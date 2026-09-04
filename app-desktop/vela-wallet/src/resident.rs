//! One Crux machine, alive for as long as the process, driven from gpui.
//!
//! [`crate::core_host::CoreHost`] already knows how to drive any machine; what it
//! cannot know is where the answers come from or when the screen redraws. This
//! module is that half, written **once and generic over the machine**, because
//! seventeen copies of an effect loop is seventeen chances to get the
//! correlation rules subtly wrong — which is the reason `core_host.rs` exists
//! once, in its own words.
//!
//! ## Why this is an entity and `session.rs` is a `Global`
//!
//! [`crate::session`] drives its machine synchronously inside a `Global`, and
//! its module doc argues the case: every session operation is one small local
//! file read, so moving it off-thread "would buy nothing measurable and would
//! introduce the one thing this module must not have — a window in which the
//! route is stale."
//!
//! The machines here are **both cases at once**. `read_store` is a local file
//! read; `probe_rpc` is a TLS round trip with a multi-second budget;
//! `start_search_debounce` is a timer armed on every keystroke. A synchronous
//! pump would freeze the window for the length of a network probe.
//!
//! And a `Global` *cannot* be the answer, for a mechanical reason rather than a
//! stylistic one: a background task has no handle through which to reach one.
//! `Entity::update` is what a spawned task calls when its answer arrives, so the
//! thing holding the core has to be an entity. That is precisely why
//! `onboarding.rs` is an entity and `session.rs` is not, and this module follows
//! the same fork for the same reason.
//!
//! ## The thread boundary is a type, not a convention
//!
//! [`Answer`] makes it structural. `Answer::Blocking` carries a boxed `FnOnce`
//! that is `Send`; `Core<A>` is not `Send` and so physically cannot be captured
//! by one. "The core never leaves the main thread" is therefore checked by the
//! compiler rather than promised in a comment.
//!
//! ```text
//!   gpui entity                 ResidentCore<A>              background executor
//!       │ dispatch(event) ───────────►│
//!       │        Answer::Now  ────────┤ resolved inline, loop until quiescent
//!       │        Answer::Blocking ────┼───────────────────────────►│
//!       │        Answer::After  ──────┼──► timer ──────────────────┤
//!       │◄────── entity.update(resolve(id, result)) ───────────────┘
//!       │ view = host.view(); cx.notify()  → observers redraw
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::time::Duration;

use crux_core::App as CruxApp;
use gpui::{App, AppContext as _, Context, Entity, Global};

use vela_core::app::SplitEffect;

use crate::core_host::CoreHost;

type Op<A> = <<A as CruxApp>::Effect as SplitEffect>::Op;
type Out<A> = <Op<A> as crux_core::capability::Operation>::Output;

/// How an operation is performed, and therefore where.
pub enum Answer<T> {
    /// A local read or write. Resolved on this thread, this frame — the
    /// `session.rs` argument, kept for the operations it is actually true of.
    Now(T),
    /// Blocks: a socket, TLS, a disk the OS decides to think about. Runs on
    /// gpui's background executor.
    ///
    /// The closure is the thread boundary. It can only capture what the
    /// *operation* owns, and `Core<A>` is not `Send`, so the core cannot cross.
    Blocking(Box<dyn FnOnce() -> T + Send>),
    /// Answered after a delay — a debounce, a backoff. Uses gpui's timer rather
    /// than a parked thread: `ShellOperation::Wait` sleeps a background thread
    /// and that is right for a flow that waits a handful of times, but a search
    /// debounce fires on every keystroke and must not cost a thread each.
    After(Duration, T),
}

/// What varies per machine — and deliberately nothing else.
pub trait Machine: CruxApp + Default + 'static
where
    Self::Model: Default,
    Self::Effect: SplitEffect,
{
    /// Named in the one log line a core fault produces.
    const LABEL: &'static str;

    /// The event that starts this machine.
    ///
    /// Takes `&App` because some machines must be told *whose* data they are
    /// about at boot: `contacts` needs the signed-in address, and the core's own
    /// comment warns that missing it "crosses the books" between accounts.
    fn boot_event(cx: &App) -> Self::Event;

    /// Perform one operation. Never fails outward — see `executor::mod`'s
    /// failure contract, which this inherits wholesale.
    fn perform(operation: &Op<Self>) -> Answer<Out<Self>>;
}

/// One machine and its latest view.
pub struct ResidentCore<A: Machine>
where
    A::Model: Default,
    A::Effect: SplitEffect,
{
    host: CoreHost<A>,
    view: A::ViewModel,
}

impl<A> ResidentCore<A>
where
    A: Machine,
    A::Model: Default,
    A::Effect: SplitEffect,
    Op<A>: Clone + 'static,
    Out<A>: 'static,
    A::ViewModel: Clone,
{
    fn new() -> Self {
        let host = CoreHost::<A>::new();
        let view = host.view();
        Self { host, view }
    }

    /// The current view. Cheap to clone; screens read it per frame.
    pub fn view(&self) -> A::ViewModel {
        self.view.clone()
    }

    /// Send an event and perform whatever it asks for.
    pub fn dispatch(&mut self, event: A::Event, cx: &mut Context<Self>) {
        let pending = self.host.dispatch(event);
        self.pump(pending, cx);
    }

    fn resolve(&mut self, id: u64, result: Out<A>, cx: &mut Context<Self>) {
        let pending = self.host.resolve(id, result);
        self.pump(pending, cx);
    }

    fn pump(&mut self, pending: Vec<crate::core_host::Pending<Op<A>>>, cx: &mut Context<Self>) {
        for next in pending {
            let id = next.id;
            match A::perform(&next.operation) {
                // Straight back into the core. Recursion depth is the machine's
                // own chain of local operations, which is short by construction.
                Answer::Now(result) => self.resolve(id, result, cx),
                Answer::Blocking(work) => {
                    cx.spawn(async move |resident, cx| {
                        let result = cx.background_executor().spawn(async move { work() }).await;
                        resident
                            .update(cx, |resident, cx| resident.resolve(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                Answer::After(delay, result) => {
                    cx.spawn(async move |resident, cx| {
                        cx.background_executor().timer(delay).await;
                        resident
                            .update(cx, |resident, cx| resident.resolve(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
            }
        }
        self.view = self.host.view();
        cx.notify();
    }
}

/// Every resident machine in this process, keyed by its own type.
///
/// A `HashMap<TypeId, …>` rather than a struct with a field per machine, and
/// that is load-bearing rather than tidy: named fields would mean wiring the
/// third machine necessarily edits this file, and "wiring the third machine
/// touches no shared plumbing" is the measurement this cut exists to produce
/// (spec 030 SC-004). Adding a machine must cost zero lines here.
#[derive(Default)]
struct Residents(HashMap<TypeId, Box<dyn Any>>);

impl Global for Residents {}

/// The resident for `A`, booting it on first use.
pub fn resident<A>(cx: &mut App) -> Entity<ResidentCore<A>>
where
    A: Machine,
    A::Model: Default,
    A::Effect: SplitEffect,
    Op<A>: Clone + 'static,
    Out<A>: 'static,
    A::ViewModel: Clone,
{
    if let Some(existing) = cx
        .try_global::<Residents>()
        .and_then(|residents| residents.0.get(&TypeId::of::<A>()))
        .and_then(|any| any.downcast_ref::<Entity<ResidentCore<A>>>())
    {
        return existing.clone();
    }

    // Read what the boot event needs BEFORE taking the entity's borrow.
    let event = A::boot_event(cx);
    eprintln!("[vela-wallet] core: {} booting", A::LABEL);
    let entity = cx.new(|_| ResidentCore::<A>::new());
    entity.update(cx, |resident, cx| resident.dispatch(event, cx));

    if !cx.has_global::<Residents>() {
        cx.set_global(Residents::default());
    }
    cx.global_mut::<Residents>()
        .0
        .insert(TypeId::of::<A>(), Box::new(entity.clone()));
    entity
}

/// Forget every machine.
///
/// Called on sign-out. Contacts, networks and the chosen currency belong to the
/// ACCOUNT, and a resident that outlived a sign-out would show the previous
/// person's address book to the next one.
pub fn drop_all(cx: &mut App) {
    if cx.has_global::<Residents>() {
        cx.global_mut::<Residents>().0.clear();
    }
}
