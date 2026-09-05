//! The send host: one `send` machine and one `fee_policy` machine, alive for
//! one journey through the send flow, driven from gpui.
//!
//! ## Why this is not two residents
//!
//! [`crate::resident`] hosts a machine for the life of the process and knows
//! nothing about its neighbours. The send journey needs the opposite on both
//! counts: its machines are born when the flow opens and discarded when it
//! closes (a second send starts from a fresh machine, not a resumed one), and
//! the two must talk — `send` asks `EstimateFee` and the answer is whatever the
//! `fee_policy` session settles on. That session is ONE object per surface:
//! the quote the core pre-checks against, the quote the confirm card shows
//! and the quote that is signed are the same estimate with the same owner.
//! The web tier records four integrations that failed by splitting it.
//!
//! So this host does what [`crate::onboarding::OnboardingPage`] does for the
//! create and login machines: holds the cores, performs their effects, runs
//! the ceremony channel's poll, and owns the few operations that belong to a
//! screen rather than to an executor.
//!
//! ## What the screen owns
//!
//! - `EstimateFee` → a deployment read, then `QuoteRequested` on the fee
//!   session, answered when that session's view settles (`busy` false, a fee
//!   or a failure). The web's `FeeQuote.requestQuote`, without the promise.
//! - `TrackSubmitted` → the app-resident tracker, whose view this host
//!   observes and forwards as `ReceiptUpdate` — only the three verdicts the
//!   core accepts.
//! - `ShowAlert` → a kind the panel words; `Close` → a flag the column reads.
//! - `SigningStarted` → raised by the sign closure the instant the prompt
//!   opens, seen by the poll, dispatched once.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use gpui::{Context, Entity, FocusHandle};

use vela_core::app::Account;
use vela_core::app::fee_policy::{
    Event as FeeEvent, FeeFailure, FeeOperation, FeePolicy, FeeShellResult, FeeTier, FeeView,
};
use vela_core::app::send::{
    Event as SendEvent, Send, SendAccountRef, SendAlertKind, SendDisplayContext,
    SendEstimateFailure, SendFeeOutcome, SendOpenParams, SendOperation, SendReceiptOutcome,
    SendShellResult, SendView,
};
use vela_core::app::tx_tracker::{TrackStatus, TxTracker};

use crate::ceremony::CeremonyChannel;
use crate::core_host::{CoreHost, Pending};
use crate::ctap::usb::TouchRequest;
use crate::executor::passkey::{CredentialChoice, PinRequest, WindowHandle};
use crate::executor::send::{self as send_executor, SendAnswer, SendContext};
use crate::executor::{chain, storage, tracker};
use crate::resident::{self, Answer, Machine, ResidentCore};

/// How often the ceremony channel and the signing flag are polled while a
/// machine is busy. The same cadence onboarding uses.
const TICK_MS: u64 = 120;

/// The PIN dialog a security key raised mid-signature.
pub struct PinDialog {
    pub request: PinRequest,
    pub value: String,
    pub focus: FocusHandle,
}

pub struct SendHost {
    send: CoreHost<Send>,
    pub view: SendView,
    fee: CoreHost<FeePolicy>,
    pub fee_view: FeeView,
    ctx: SendContext,
    channel: Arc<CeremonyChannel>,
    window_handle: WindowHandle,
    /// The `EstimateFee` effect the fee session is answering.
    pending_fee: Option<u64>,
    /// Guards the deployment read: a slower one must not dispatch a quote
    /// for a request that has been superseded.
    fee_seq: u64,
    last_fee_busy: bool,
    last_fee_chain: Option<(u32, String)>,
    /// `ShowAlert`'s kind, until the panel acknowledges it.
    pub alert: Option<SendAlertKind>,
    /// The core asked to leave the flow.
    pub closed: bool,
    pub pin: Option<PinDialog>,
    pub pick: Option<Vec<CredentialChoice>>,
    watching: bool,
    signing_reported: bool,
    tracked_hash: Option<String>,
    last_track_status: Option<TrackStatus>,
}

/// The active account, whole — the send flow signs as it.
pub fn active_account() -> Option<Account> {
    let accounts = storage::load_accounts().ok()?;
    accounts.into_iter().nth(storage::load_active_index())
}

impl SendHost {
    pub fn open(
        account: Account,
        params: SendOpenParams,
        display: SendDisplayContext,
        window_handle: WindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let channel = CeremonyChannel::new();
        let ctx = SendContext::new(&account, channel.ceremony(window_handle));
        let send = CoreHost::<Send>::new();
        let fee = CoreHost::<FeePolicy>::new();
        let view = send.view();
        let fee_view = fee.view();
        let mut host = Self {
            send,
            view,
            fee,
            fee_view,
            ctx,
            channel,
            window_handle,
            pending_fee: None,
            fee_seq: 0,
            last_fee_busy: false,
            last_fee_chain: None,
            alert: None,
            closed: false,
            pin: None,
            pick: None,
            watching: false,
            signing_reported: false,
            tracked_hash: None,
            last_track_status: None,
        };

        // Receipts arrive through the app-resident tracker, which outlives
        // this host; observing it is what turns a confirmation into the
        // receipt screen's state.
        let tracked = resident::resident::<TxTracker>(cx);
        cx.observe(&tracked, |host, tracked, cx| host.on_tracker(&tracked, cx))
            .detach();

        host.dispatch(
            SendEvent::Open {
                account: Some(SendAccountRef {
                    id: account.id.clone(),
                    address: account.address.clone(),
                    name: (!account.name.is_empty()).then(|| account.name.clone()),
                }),
                params,
                display,
            },
            cx,
        );
        host
    }

    // -- the two machines ----------------------------------------------------

    pub fn dispatch(&mut self, event: SendEvent, cx: &mut Context<Self>) {
        let pending = self.send.dispatch(event);
        self.pump_send(pending, cx);
    }

    pub fn fee_dispatch(&mut self, event: FeeEvent, cx: &mut Context<Self>) {
        let pending = self.fee.dispatch(event);
        self.pump_fee(pending, cx);
    }

    fn resolve_send(&mut self, id: u64, result: SendShellResult, cx: &mut Context<Self>) {
        let pending = self.send.resolve(id, result);
        self.pump_send(pending, cx);
    }

    fn resolve_fee(&mut self, id: u64, result: FeeShellResult, cx: &mut Context<Self>) {
        let pending = self.fee.resolve(id, result);
        self.pump_fee(pending, cx);
    }

    fn pump_send(&mut self, pending: Vec<Pending<SendOperation>>, cx: &mut Context<Self>) {
        for effect in pending {
            self.perform_send(effect, cx);
        }
        self.view = self.send.view();
        self.ensure_watcher(cx);
        cx.notify();
    }

    fn pump_fee(&mut self, pending: Vec<Pending<FeeOperation>>, cx: &mut Context<Self>) {
        for effect in pending {
            let id = effect.id;
            match <FeePolicy as Machine>::perform(&effect.operation) {
                Answer::Now(result) => self.resolve_fee(id, result, cx),
                Answer::Blocking(work) => {
                    cx.spawn(async move |host, cx| {
                        let result = cx.background_executor().spawn(async move { work() }).await;
                        host.update(cx, |host, cx| host.resolve_fee(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                Answer::After(delay, result) => {
                    cx.spawn(async move |host, cx| {
                        cx.background_executor().timer(delay).await;
                        host.update(cx, |host, cx| host.resolve_fee(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
            }
        }
        self.fee_view = self.fee.view();
        self.sync_fee_to_send(cx);
        self.settle_fee(cx);
        self.ensure_watcher(cx);
        cx.notify();
    }

    /// Start one send operation. The four screen-owned arms are performed
    /// here; everything else goes to the executor and, when it blocks, to
    /// the background executor with the answer routed back by effect id.
    fn perform_send(&mut self, effect: Pending<SendOperation>, cx: &mut Context<Self>) {
        let id = effect.id;
        match &effect.operation {
            SendOperation::EstimateFee {
                chain_id,
                account,
                tx,
                batch,
                gas_fee_token,
                public_key_hex,
            } => {
                // A batch takes precedence only when it HAS legs; an empty
                // one would otherwise silence the single call beside it.
                let calls = match (batch, tx) {
                    (Some(batch), _) if !batch.is_empty() => batch.clone(),
                    (_, Some(tx)) => vec![tx.clone()],
                    _ => Vec::new(),
                };
                self.request_quote(
                    id,
                    *chain_id,
                    account.clone(),
                    calls,
                    gas_fee_token.clone(),
                    public_key_hex.is_some(),
                    cx,
                );
                return;
            }
            SendOperation::TrackSubmitted {
                user_op_hash,
                record_ids,
                chain_id,
            } => {
                self.tracked_hash = Some(user_op_hash.to_lowercase());
                self.last_track_status = None;
                tracker::submitted(user_op_hash.clone(), record_ids.clone(), *chain_id, cx);
                self.resolve_send(id, SendShellResult::TrackHandedOff, cx);
                return;
            }
            SendOperation::ShowAlert { kind } => {
                self.alert = Some(kind.clone());
                self.resolve_send(id, SendShellResult::AlertAcknowledged, cx);
                return;
            }
            SendOperation::Close => {
                self.closed = true;
                self.resolve_send(id, SendShellResult::Closed, cx);
                return;
            }
            // Closing the channel is what cancels a waiting ceremony; a fresh
            // one is opened for the next attempt. The executor owes the ack.
            SendOperation::CancelPasskeySign => self.cancel_ceremony(),
            SendOperation::SubmitUserOp { .. } => {
                self.ctx.signing_started.store(false, Ordering::SeqCst);
                self.signing_reported = false;
            }
            _ => {}
        }

        match send_executor::perform(&effect.operation, &self.ctx) {
            SendAnswer::Now(result) => self.resolve_send(id, result, cx),
            SendAnswer::Blocking(work) => {
                cx.spawn(async move |host, cx| {
                    let result = cx.background_executor().spawn(async move { work() }).await;
                    host.update(cx, |host, cx| host.resolve_send(id, result, cx))
                        .ok();
                })
                .detach();
            }
            SendAnswer::After(delay, result) => {
                cx.spawn(async move |host, cx| {
                    cx.background_executor().timer(delay).await;
                    host.update(cx, |host, cx| host.resolve_send(id, result, cx))
                        .ok();
                })
                .detach();
            }
            // Every screen-owned operation returned above; nothing reaches
            // here by construction.
            SendAnswer::Screen => {}
        }
    }

    // -- the fee session ------------------------------------------------------

    /// `FeeQuote.requestQuote`: read the deployment status (never guessed),
    /// then ask the session; the answer arrives when its view settles.
    #[allow(clippy::too_many_arguments, reason = "the operation's own fields")]
    fn request_quote(
        &mut self,
        effect_id: u64,
        chain_id: u32,
        account: String,
        calls: Vec<vela_core::app::fee_policy::FeeCall>,
        fee_token: Option<String>,
        public_key_available: bool,
        cx: &mut Context<Self>,
    ) {
        // A newer question supersedes the last inside the core; its asker is
        // answered with a refusal rather than left waiting forever.
        if let Some(previous) = self.pending_fee.take() {
            self.resolve_send(previous, estimate_failed(), cx);
        }
        self.pending_fee = Some(effect_id);
        self.fee_seq += 1;
        let seq = self.fee_seq;
        cx.spawn(async move |host, cx| {
            let deployed = cx
                .background_executor()
                .spawn(async move { chain::is_deployed(&account, chain_id).map(|d| (d, account)) })
                .await;
            host.update(cx, |host, cx| {
                if seq != host.fee_seq {
                    return;
                }
                match deployed {
                    // An indeterminate read never reaches the core: guessing
                    // "deployed" ships an op without initCode, guessing
                    // "undeployed" attaches one to a live account.
                    Err(_) => {
                        if let Some(id) = host.pending_fee.take() {
                            host.resolve_send(id, estimate_failed(), cx);
                        }
                    }
                    Ok((deployed, account)) => host.fee_dispatch(
                        FeeEvent::QuoteRequested {
                            chain_id,
                            account,
                            deployed,
                            public_key_available,
                            tier: FeeTier::Fast,
                            calls,
                            fee_token,
                        },
                        cx,
                    ),
                }
            })
            .ok();
        })
        .detach();
    }

    /// The fee session's view, as the send machine's answer — judged against
    /// the SAME view the confirm card renders, so the two cannot disagree.
    fn settle_fee(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.pending_fee else {
            return;
        };
        if self.fee_view.busy {
            return;
        }
        let outcome = if let Some(estimate) = &self.fee_view.fee {
            SendFeeOutcome::Ok {
                estimate: estimate.clone(),
            }
        } else if let Some(failure) = self.fee_view.failed {
            SendFeeOutcome::Failed {
                kind: map_failure(failure),
            }
        } else {
            // The session moved on under the question — the web's "abandoned".
            return;
        };
        self.pending_fee = None;
        self.resolve_send(id, SendShellResult::FeeEstimated { outcome }, cx);
    }

    /// The card's re-quotes, mirrored into the send machine: `busy` flips
    /// disarm the confirm slide, and a settled estimate replaces the one it
    /// pre-checked with (`GasFeeCard.onBusyChange` / `onFeeUpdate`).
    fn sync_fee_to_send(&mut self, cx: &mut Context<Self>) {
        let busy = self.fee_view.busy;
        if busy != self.last_fee_busy {
            self.last_fee_busy = busy;
            self.dispatch(SendEvent::FeeBusyChanged { busy }, cx);
        }
        if let Some(fee) = &self.fee_view.fee {
            let stamp = (
                fee.chain_id,
                format!("{}:{:?}", fee.total_wei, fee.fee_asset),
            );
            if self.last_fee_chain.as_ref() != Some(&stamp) {
                self.last_fee_chain = Some(stamp);
                self.dispatch(
                    SendEvent::FeeUpdated {
                        estimate: fee.clone(),
                    },
                    cx,
                );
            }
        }
    }

    // -- the tracker ----------------------------------------------------------

    fn on_tracker(&mut self, tracked: &Entity<ResidentCore<TxTracker>>, cx: &mut Context<Self>) {
        let Some(hash) = self.tracked_hash.clone() else {
            return;
        };
        let view = tracked.read(cx).view();
        let Some(entry) = view
            .entries
            .iter()
            .find(|entry| entry.user_op_hash.eq_ignore_ascii_case(&hash))
        else {
            return;
        };
        if self.last_track_status == Some(entry.status) {
            return;
        }
        self.last_track_status = Some(entry.status);
        // Only the three verdicts `ReceiptUpdate` accepts; a slow or
        // unreachable poll sends nothing (invariant ⑤).
        let outcome = match entry.status {
            TrackStatus::Confirmed => SendReceiptOutcome::Confirmed {
                tx_hash: entry.tx_hash.clone().unwrap_or_default(),
            },
            TrackStatus::Dropped => SendReceiptOutcome::Failed { rejected: false },
            TrackStatus::Rejected => SendReceiptOutcome::Failed { rejected: true },
            TrackStatus::FeeHeld => SendReceiptOutcome::FeeHeld,
            TrackStatus::Pending | TrackStatus::Unreachable | TrackStatus::AcceptedNotLanded => {
                return;
            }
        };
        self.dispatch(
            SendEvent::ReceiptUpdate {
                user_op_hash: entry.user_op_hash.clone(),
                outcome,
            },
            cx,
        );
    }

    // -- the ceremony ---------------------------------------------------------

    fn cancel_ceremony(&mut self) {
        self.channel.close();
        self.channel = CeremonyChannel::new();
        self.ctx.ceremony = self.channel.ceremony(self.window_handle);
        self.pin = None;
        self.pick = None;
    }

    /// Poll the ceremony channel and the signing flag while anything is in
    /// flight. Detached; ends by returning.
    fn ensure_watcher(&mut self, cx: &mut Context<Self>) {
        if self.watching || (self.send.is_idle() && self.fee.is_idle()) {
            return;
        }
        self.watching = true;
        cx.spawn(async move |host, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(TICK_MS))
                    .await;
                let keep_going = host.update(cx, |host, cx| host.tick(cx)).unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// One poll. Returns whether to keep polling.
    fn tick(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.signing_reported && self.ctx.signing_started.load(Ordering::SeqCst) {
            self.signing_reported = true;
            self.dispatch(SendEvent::SigningStarted, cx);
        }
        if let Some(request) = self.channel.pending_pin() {
            if self
                .pin
                .as_ref()
                .is_none_or(|open| open.request.retry != request.retry)
            {
                self.pin = Some(PinDialog {
                    request,
                    value: String::new(),
                    focus: cx.focus_handle(),
                });
            }
        } else if self.pin.is_some() {
            self.pin = None;
        }
        let asking = self.channel.pending_choice();
        if self.pick.is_some() != asking.is_some() {
            self.pick = asking;
        }
        let busy = !self.send.is_idle() || !self.fee.is_idle();
        cx.notify();
        if !busy {
            self.watching = false;
        }
        busy
    }

    /// What the key is waiting for right now, if anything.
    pub fn touch_waiting(&self) -> Option<TouchRequest> {
        self.channel.touch_waiting()
    }

    /// The caBLE QR to show, if a hybrid ceremony waits for a scan.
    pub fn qr_showing(&self) -> Option<String> {
        self.channel.qr_showing()
    }

    pub fn answer_pin(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.channel.answer_pin(value);
        self.pin = None;
        cx.notify();
    }

    pub fn answer_choice(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.channel.answer_choice(index);
        self.pick = None;
        cx.notify();
    }

    /// The panel showed the alert.
    pub fn acknowledge_alert(&mut self, cx: &mut Context<Self>) {
        self.alert = None;
        cx.notify();
    }
}

fn estimate_failed() -> SendShellResult {
    SendShellResult::FeeEstimated {
        outcome: SendFeeOutcome::Failed {
            kind: SendEstimateFailure::EstimateFailed,
        },
    }
}

/// The fee vocabulary IS the send vocabulary, one name at a time.
fn map_failure(failure: FeeFailure) -> SendEstimateFailure {
    match failure {
        FeeFailure::MissingPublicKey => SendEstimateFailure::MissingPublicKey,
        FeeFailure::FeeTokenUnavailable => SendEstimateFailure::FeeTokenUnavailable,
        FeeFailure::QuoteUnavailable => SendEstimateFailure::QuoteUnavailable,
        FeeFailure::CalculationFailed => SendEstimateFailure::CalculationFailed,
        FeeFailure::EstimateFailed => SendEstimateFailure::EstimateFailed,
        FeeFailure::GasQuoteTooHigh => SendEstimateFailure::GasQuoteTooHigh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host without gpui: both machines pumped to quiescence on this
    /// thread, every blocking answer performed inline, the two timers left
    /// deliberately unanswered (the 15 s estimate race must be won by the
    /// estimate, and the quote's TTL is advisory). What `SendHost` does on the
    /// main thread, minus the thread.
    #[cfg(feature = "dev-fixtures")]
    struct SyncMoney {
        send: CoreHost<Send>,
        fee: CoreHost<FeePolicy>,
        ctx: SendContext,
        alerts: Vec<SendAlertKind>,
        submitted: Vec<String>,
    }

    #[cfg(feature = "dev-fixtures")]
    impl SyncMoney {
        fn dispatch(&mut self, event: SendEvent) {
            let pending = self.send.dispatch(event);
            self.pump(pending);
        }

        fn view(&self) -> SendView {
            self.send.view()
        }

        fn pump(&mut self, mut pending: Vec<Pending<SendOperation>>) {
            while let Some(effect) = pending.pop() {
                let id = effect.id;
                let result = match &effect.operation {
                    SendOperation::EstimateFee {
                        chain_id,
                        account,
                        tx,
                        batch,
                        gas_fee_token,
                        public_key_hex,
                    } => {
                        let calls = match (batch, tx) {
                            (Some(batch), _) if !batch.is_empty() => batch.clone(),
                            (_, Some(tx)) => vec![tx.clone()],
                            _ => Vec::new(),
                        };
                        let deployed = chain::is_deployed(account, *chain_id)
                            .unwrap_or_else(|e| unreachable!("{e}"));
                        let fee_pending = self.fee.dispatch(FeeEvent::QuoteRequested {
                            chain_id: *chain_id,
                            account: account.clone(),
                            deployed,
                            public_key_available: public_key_hex.is_some(),
                            tier: FeeTier::Fast,
                            calls,
                            fee_token: gas_fee_token.clone(),
                        });
                        self.pump_fee(fee_pending);
                        let view = self.fee.view();
                        assert!(!view.busy, "the fee session settles synchronously here");
                        let outcome = match (&view.fee, view.failed) {
                            (Some(estimate), _) => SendFeeOutcome::Ok {
                                estimate: estimate.clone(),
                            },
                            (None, Some(failure)) => SendFeeOutcome::Failed {
                                kind: map_failure(failure),
                            },
                            (None, None) => {
                                unreachable!("a settled session has a fee or a failure")
                            }
                        };
                        SendShellResult::FeeEstimated { outcome }
                    }
                    SendOperation::TrackSubmitted { user_op_hash, .. } => {
                        self.submitted.push(user_op_hash.clone());
                        SendShellResult::TrackHandedOff
                    }
                    SendOperation::ShowAlert { kind } => {
                        self.alerts.push(kind.clone());
                        SendShellResult::AlertAcknowledged
                    }
                    SendOperation::Close => SendShellResult::Closed,
                    // The estimate race: answering the timer first would make
                    // every estimate a timeout. Left pending on purpose.
                    SendOperation::StartTimer { .. } => continue,
                    _ => match send_executor::perform(&effect.operation, &self.ctx) {
                        SendAnswer::Now(result) => result,
                        SendAnswer::Blocking(work) => work(),
                        SendAnswer::After(..) => continue,
                        SendAnswer::Screen => unreachable!("every screen arm is matched above"),
                    },
                };
                pending.extend(self.send.resolve(id, result));
            }
        }

        fn pump_fee(&mut self, mut pending: Vec<Pending<FeeOperation>>) {
            while let Some(effect) = pending.pop() {
                let id = effect.id;
                let result = match <FeePolicy as Machine>::perform(&effect.operation) {
                    Answer::Now(result) => result,
                    Answer::Blocking(work) => work(),
                    // The TTL is advisory; a test does not wait thirty seconds.
                    Answer::After(..) => continue,
                };
                pending.extend(self.fee.resolve(id, result));
            }
        }
    }

    /// The golden multi-key Safe, seeded as the active account from the
    /// fixture keyset — the wallet every 031 live sweep read from.
    #[cfg(feature = "dev-fixtures")]
    fn seed_golden_account() -> Account {
        use vela_core::app::AccountKey;
        use vela_core::dev_fixtures as fixtures;
        let keys = fixtures::accounts().unwrap_or_else(|e| unreachable!("{e}"));
        let account = Account {
            id: keys[0].credential_id_hex.clone(),
            name: "Golden".to_owned(),
            address: fixtures::multi_address().unwrap_or_else(|e| unreachable!("{e}")),
            public_key_hex: keys[0].public_key_hex.clone(),
            created_at_iso: "2026-09-05T00:00:00.000Z".to_owned(),
            keys: keys
                .iter()
                .map(|key| AccountKey {
                    credential_id: key.credential_id_hex.clone(),
                    public_key_hex: key.public_key_hex.clone(),
                    name: key.name.to_owned(),
                    transports: String::new(),
                })
                .collect(),
        };
        storage::save_account(&account).unwrap_or_else(|e| unreachable!("{e}"));
        storage::save_active_index(0).unwrap_or_else(|e| unreachable!("{e}"));
        account
    }

    /// The whole spine up to the confirm screen, live, for the golden Safe:
    /// its real holdings load, XDAI on Gnosis is picked, a dust transfer to
    /// fixture #2 is drafted, and `Continue` brings back the relay's real
    /// quote — the stage is Confirm and the slide is armed. No signature, no
    /// submit: nothing moves.
    ///
    /// `VELA_LIVE_SEND=1` goes one step further and slides — the parallel
    /// space's fixture #1 signs, the relay accepts, and the hash is printed.
    /// That step spends dust and is never run by default.
    #[cfg(feature = "dev-fixtures")]
    #[test]
    #[ignore = "real network; VELA_LIVE_SEND=1 spends dust"]
    fn live_the_golden_safe_reaches_confirm_with_a_real_quote() {
        crate::executor::storage::tests::with_temp_state("money-live", || {
            let account = seed_golden_account();
            let ceremony = CeremonyChannel::new().ceremony(0);
            let ctx = SendContext::new(&account, ceremony);
            let mut money = SyncMoney {
                send: CoreHost::<Send>::new(),
                fee: CoreHost::<FeePolicy>::new(),
                ctx,
                alerts: Vec::new(),
                submitted: Vec::new(),
            };
            money.dispatch(SendEvent::Open {
                account: Some(SendAccountRef {
                    id: account.id.clone(),
                    address: account.address.clone(),
                    name: Some(account.name.clone()),
                }),
                params: SendOpenParams::default(),
                display: SendDisplayContext::default(),
            });
            let view = money.view();
            println!("tokens: {}", view.tokens.len());
            for token in &view.tokens {
                println!(
                    "  {} on {} = {}",
                    token.symbol, token.chain_id, token.balance
                );
            }
            let xdai = view
                .tokens
                .iter()
                .find(|token| token.chain_id == 100 && token.token_address.is_none())
                .unwrap_or_else(|| unreachable!("the golden Safe holds xDAI on Gnosis"));
            assert!(
                money.alerts.is_empty(),
                "no alert on open: {:?}",
                money.alerts
            );

            money.dispatch(SendEvent::SelectToken {
                token_id: xdai.id(),
            });
            assert_eq!(
                money.view().stage,
                vela_core::app::send::SendStage::EnterDetails
            );

            let to = vela_core::dev_fixtures::account(1)
                .unwrap_or_else(|e| unreachable!("{e}"))
                .address;
            money.dispatch(SendEvent::SetRecipient {
                recipient: to.clone(),
            });
            money.dispatch(SendEvent::SetAmount {
                amount: "0.001".to_owned(),
            });
            let view = money.view();
            println!(
                "draft: to={to} amount={} token_amount={} warning={:?} can_continue={}",
                view.amount, view.token_amount, view.amount_warning, view.can_continue
            );
            assert!(view.can_continue, "the draft is sendable");

            money.dispatch(SendEvent::Continue);
            let view = money.view();
            println!(
                "after continue: stage={:?} fee={:?} fee_busy={} treasury={:?} alerts={:?} can_confirm={}",
                view.stage,
                view.fee.as_ref().map(|fee| (
                    &fee.total_wei,
                    &fee.fee_asset,
                    fee.fee_recipient.clone()
                )),
                view.fee_busy,
                view.treasury_bootstrap,
                money.alerts,
                view.can_confirm
            );
            assert_eq!(view.stage, vela_core::app::send::SendStage::Confirm);
            let fee = view
                .fee
                .clone()
                .unwrap_or_else(|| unreachable!("a real quote"));
            assert_eq!(fee.chain_id, 100);
            assert!(fee.quoted, "the relay's own quote, not a local fallback");
            assert!(view.can_confirm, "the slide is armed");

            if std::env::var("VELA_LIVE_SEND").as_deref() != Ok("1") {
                println!("stopping before the slide: set VELA_LIVE_SEND=1 to spend dust");
                return;
            }
            money.dispatch(SendEvent::SlideConfirm);
            let view = money.view();
            println!(
                "after slide: tx_status={:?} error={:?} user_op_hash={:?} tracked={:?}",
                view.tx_status, view.tx_error, view.user_op_hash, money.submitted
            );
            assert!(
                view.user_op_hash.is_some(),
                "the relay accepted the operation"
            );
        });
    }

    #[test]
    fn the_fee_vocabulary_maps_one_to_one() {
        assert_eq!(
            map_failure(FeeFailure::GasQuoteTooHigh),
            SendEstimateFailure::GasQuoteTooHigh
        );
        assert_eq!(
            map_failure(FeeFailure::MissingPublicKey),
            SendEstimateFailure::MissingPublicKey
        );
        assert!(matches!(
            estimate_failed(),
            SendShellResult::FeeEstimated {
                outcome: SendFeeOutcome::Failed {
                    kind: SendEstimateFailure::EstimateFailed
                }
            }
        ));
    }

    /// The active account is the one the session index names, whole — keys
    /// included, so a multi-key wallet signs as itself.
    #[test]
    fn the_active_account_is_read_whole() {
        crate::executor::storage::tests::with_temp_state("money-active", || {
            assert!(active_account().is_none());
            let mut account = Account {
                id: "cred0".to_owned(),
                name: "Wallet".to_owned(),
                address: "0x0000000000000000000000000000000000000001".to_owned(),
                public_key_hex: "04aa".to_owned(),
                created_at_iso: String::new(),
                keys: Vec::new(),
            };
            let _ = storage::save_account(&account);
            account.id = "cred1".to_owned();
            account.address = "0x0000000000000000000000000000000000000002".to_owned();
            let _ = storage::save_account(&account);
            let _ = storage::save_active_index(1);
            assert_eq!(
                active_account().map(|a| a.address).as_deref(),
                Some("0x0000000000000000000000000000000000000002")
            );
        });
    }
}
