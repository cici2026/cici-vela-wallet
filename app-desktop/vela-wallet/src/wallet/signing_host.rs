//! The signing panel's journey — four machines, one column.
//!
//! `sign_request` owns the request's life; `clear_signing` says what the
//! transaction DOES; `approval_guard` decides what an approval may be edited
//! to; `fee_policy` prices it. The sheet reads all four, which is why they are
//! born and die together here rather than as residents: a request that ended
//! must not leave a decoded intent or a half-edited allowance behind for the
//! next one to inherit.
//!
//! The shape is `SendHost`'s, deliberately — the send column already proved
//! that a journey across machines wants one owner with one pump per machine,
//! and the correlation rules it got wrong four times (spec 032 lesson 2, the
//! fee session that must be ONE session) are not worth rediscovering.
//!
//! ## Who answers the dApp
//!
//! `SendResponse` is `SignAnswer::Screen` in the executor because only this
//! layer knows the transport a request arrived on. Today there is one — the
//! browser column — so the answer goes to `webview::deliver`. When there are
//! two, the `transport_id` the core carries is what picks between them, and
//! that id exists precisely so a response can never go to the wrong site.

use std::sync::Arc;

use futures::StreamExt as _;
use gpui::Context;

use vela_core::app::Account;
use vela_core::app::approval_guard::{
    ApprovalGuard, Event as GuardEvent, GuardOperation, GuardShellResult, GuardView,
};
use vela_core::app::clear_signing::{
    ClearOperation, ClearShellResult, ClearSigning, ClearSigningView, Event as ClearEvent,
};
use vela_core::app::sign_request::{
    Event as SignEvent, SignOperation, SignRequest, SignShellResult, SignView,
};

use crate::ceremony::CeremonyChannel;
use crate::core_host::{CoreHost, Pending};
use crate::executor::now_ms;
use crate::executor::passkey::WindowHandle;
use crate::executor::sign_request::{self as sign_executor, SignAnswer, SignContext};
use crate::resident::{Answer, Machine, Sink};

/// One dApp request, as the shell received it.
pub struct IncomingRequest {
    pub id: String,
    pub method: String,
    /// Raw JSON-RPC params, verbatim and untrusted.
    pub params_json: String,
    /// Read from the TRANSPORT, never from the page (`webview.rs`'s rule).
    pub origin: String,
    pub transport_id: String,
    pub chain_id: u32,
}

pub struct SigningHost {
    sign: CoreHost<SignRequest>,
    pub view: SignView,
    clear: CoreHost<ClearSigning>,
    pub clear_view: ClearSigningView,
    guard: CoreHost<ApprovalGuard>,
    pub guard_view: GuardView,
    /// The fourth view the sheet reads — and the one machine of the four that
    /// is NOT running yet.
    ///
    /// A pristine `fee_policy` answers `confirm_fee_ready: false`, so the
    /// slide stays shut while this is unwired. That is the right failure —
    /// arming a confirm over a price nobody obtained is the specific thing
    /// the three-way AND exists to prevent — but it is a HELD view rather
    /// than one rebuilt per frame, so the day the session lands there is one
    /// place to attach it, and only one (this cut's second lesson: the fee
    /// session must be one session).
    pub fee_view: vela_core::app::fee_policy::FeeView,
    ctx: SignContext,
    #[allow(dead_code, reason = "held so the ceremony outlives the request")]
    channel: Arc<CeremonyChannel>,
    /// The core asked to close the column.
    pub closed: bool,
}

impl SigningHost {
    pub fn open(
        account: &Account,
        request: IncomingRequest,
        window_handle: WindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let channel = CeremonyChannel::new();
        let ctx = SignContext::new(account, channel.ceremony(window_handle));
        let sign = CoreHost::<SignRequest>::new();
        let clear = CoreHost::<ClearSigning>::new();
        let guard = CoreHost::<ApprovalGuard>::new();
        // The cores' own pristine views rather than a `Default` they do not
        // have: the shell must never invent a starting shape for a machine.
        let (view, clear_view, guard_view) = (sign.view(), clear.view(), guard.view());
        let fee_view = CoreHost::<vela_core::app::fee_policy::FeePolicy>::new().view();
        let mut host = Self {
            sign,
            view,
            clear,
            clear_view,
            guard,
            guard_view,
            fee_view,
            ctx,
            channel,
            closed: false,
        };
        host.begin(&request, &account.address, cx);
        host
    }

    /// One request, told to all three machines.
    ///
    /// They are told SEPARATELY and none of them waits for the others: the
    /// decode is a network round trip and the approval editor is not, so a
    /// sheet that opened only when the slowest finished would sit blank
    /// through every descriptor fetch.
    fn begin(&mut self, request: &IncomingRequest, wallet: &str, cx: &mut Context<Self>) {
        let now = now_ms();
        self.dispatch_sign(
            SignEvent::RequestArrived {
                id: request.id.clone(),
                method: request.method.clone(),
                params_json: request.params_json.clone(),
                origin: request.origin.clone(),
                transport_id: request.transport_id.clone(),
                dedicated_transport: true,
                per_request_chain: Some(request.chain_id),
                dapp: None,
                granted_address: None,
                requested_address: None,
                request_ts_ms: None,
                now_ms: now,
            },
            cx,
        );

        // What it does. A transaction decodes from its call; typed data and a
        // plain message are their own rungs of the same ladder.
        let clear = match request.method.as_str() {
            "eth_sendTransaction" | "wallet_sendCalls" => {
                let call = first_call(&request.params_json);
                Some(ClearEvent::ResolveTransaction {
                    to: call.as_ref().and_then(|c| c.0.clone()),
                    data: call.as_ref().and_then(|c| c.1.clone()),
                    value: call.and_then(|c| c.2),
                    chain_id: request.chain_id,
                    locale: vela_core::app::clear_signing::ClearLocale::default(),
                })
            }
            "eth_signTypedData_v4" | "eth_signTypedData" => Some(ClearEvent::ResolveTypedData {
                typed_data_json: typed_data_of(&request.params_json),
                chain_id: request.chain_id,
                locale: vela_core::app::clear_signing::ClearLocale::default(),
            }),
            _ => None,
        };
        if let Some(event) = clear {
            self.dispatch_clear(event, cx);
        }

        // What it may be edited to. `read_only` is false: this sheet is the
        // one place the never-unlimited gate can be satisfied, and a guard
        // that cannot be edited would leave "unlimited" as the only option.
        self.dispatch_guard(
            GuardEvent::ApprovalDetected {
                method: request.method.clone(),
                params_json: request.params_json.clone(),
                chain_id: request.chain_id,
                wallet_address: Some(wallet.to_owned()),
                read_only: false,
                now_ms: now,
            },
            cx,
        );
    }

    pub fn dispatch_sign(&mut self, event: SignEvent, cx: &mut Context<Self>) {
        let pending = self.sign.dispatch(event);
        self.pump_sign(pending, cx);
    }

    pub fn dispatch_clear(&mut self, event: ClearEvent, cx: &mut Context<Self>) {
        let pending = self.clear.dispatch(event);
        self.pump_clear(pending, cx);
    }

    pub fn dispatch_guard(&mut self, event: GuardEvent, cx: &mut Context<Self>) {
        let pending = self.guard.dispatch(event);
        self.pump_guard(pending, cx);
    }

    fn resolve_sign(&mut self, id: u64, result: SignShellResult, cx: &mut Context<Self>) {
        let pending = self.sign.resolve(id, result);
        self.pump_sign(pending, cx);
    }

    fn resolve_clear(&mut self, id: u64, result: ClearShellResult, cx: &mut Context<Self>) {
        let pending = self.clear.resolve(id, result);
        self.pump_clear(pending, cx);
    }

    fn resolve_guard(&mut self, id: u64, result: GuardShellResult, cx: &mut Context<Self>) {
        let pending = self.guard.resolve(id, result);
        self.pump_guard(pending, cx);
    }

    fn pump_sign(&mut self, pending: Vec<Pending<SignOperation>>, cx: &mut Context<Self>) {
        for effect in pending {
            let id = effect.id;
            match sign_executor::perform(&effect.operation, &self.ctx) {
                SignAnswer::Now(result) => self.resolve_sign(id, result, cx),
                SignAnswer::Blocking(work) => {
                    cx.spawn(async move |host, cx| {
                        let result = cx.background_executor().spawn(async move { work() }).await;
                        host.update(cx, |host, cx| host.resolve_sign(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                SignAnswer::Streaming(work) => {
                    let (tx, mut rx) = futures::channel::mpsc::unbounded();
                    cx.spawn(async move |host, cx| {
                        let work = cx
                            .background_executor()
                            .spawn(async move { work(&Sink::new(tx)) });
                        // The submitted hash reaches the core BEFORE the
                        // submit settles — that ordering is the whole reason
                        // this arm exists (spec 032 phase 12).
                        while let Some(event) = rx.next().await {
                            host.update(cx, |host, cx| host.dispatch_sign(event, cx))
                                .ok();
                        }
                        let result = work.await;
                        host.update(cx, |host, cx| host.resolve_sign(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                SignAnswer::Screen => self.answer_transport(id, &effect.operation, cx),
            }
        }
        self.view = self.sign.view();
        // `Hidden` is the core saying the request is over — the column closes
        // on the machine's word, never on a click this file interpreted.
        self.closed = self.view.surface == vela_core::app::sign_request::SignSurface::Hidden;
        cx.notify();
    }

    /// The one screen-owned operation: answering the site.
    fn answer_transport(&mut self, id: u64, operation: &SignOperation, cx: &mut Context<Self>) {
        if let SignOperation::SendResponse {
            id: request_id,
            payload,
            ..
        } = operation
        {
            #[cfg(not(target_os = "linux"))]
            crate::webview::respond(request_id, payload);
            #[cfg(target_os = "linux")]
            let _ = (request_id, payload);
        }
        // Answered either way: the core sequences record-then-respond off this
        // acknowledgement, and withholding it would strand the request.
        self.resolve_sign(id, SignShellResult::Responded, cx);
    }

    fn pump_clear(&mut self, pending: Vec<Pending<ClearOperation>>, cx: &mut Context<Self>) {
        for effect in pending {
            let id = effect.id;
            match ClearSigning::perform(&effect.operation) {
                Answer::Now(result) => self.resolve_clear(id, result, cx),
                Answer::Blocking(work) => {
                    cx.spawn(async move |host, cx| {
                        let result = cx.background_executor().spawn(async move { work() }).await;
                        host.update(cx, |host, cx| host.resolve_clear(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                Answer::After(delay, result) => {
                    cx.spawn(async move |host, cx| {
                        cx.background_executor().timer(delay).await;
                        host.update(cx, |host, cx| host.resolve_clear(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                Answer::Streaming(_) => unreachable!("clear_signing reports nothing mid-flight"),
            }
        }
        self.clear_view = self.clear.view();
        cx.notify();
    }

    fn pump_guard(&mut self, pending: Vec<Pending<GuardOperation>>, cx: &mut Context<Self>) {
        for effect in pending {
            let id = effect.id;
            match ApprovalGuard::perform(&effect.operation) {
                Answer::Now(result) => self.resolve_guard(id, result, cx),
                Answer::Blocking(work) => {
                    cx.spawn(async move |host, cx| {
                        let result = cx.background_executor().spawn(async move { work() }).await;
                        host.update(cx, |host, cx| host.resolve_guard(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                Answer::After(delay, result) => {
                    cx.spawn(async move |host, cx| {
                        cx.background_executor().timer(delay).await;
                        host.update(cx, |host, cx| host.resolve_guard(id, result, cx))
                            .ok();
                    })
                    .detach();
                }
                Answer::Streaming(_) => unreachable!("approval_guard reports nothing mid-flight"),
            }
        }
        self.guard_view = self.guard.view();
        cx.notify();
    }
}

/// The first call's `to` / `data` / `value`, for the decoder.
///
/// A batch decodes from its first leg today, which is what the phone's sheet
/// shows too; the per-leg panorama (CS26) is a drawn state nobody has wired.
fn first_call(params_json: &str) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let params: serde_json::Value = serde_json::from_str(params_json).ok()?;
    let first = params.get(0)?;
    let call = first
        .get("calls")
        .and_then(|calls| calls.get(0))
        .unwrap_or(first);
    let field = |name: &str| {
        call.get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    Some((field("to"), field("data"), field("value")))
}

/// `eth_signTypedData_v4`'s payload: `[address, json]` — the JSON is the
/// SECOND parameter, and reading the first would hand the decoder an address
/// where it expects a document.
fn typed_data_of(params_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(params_json)
        .ok()
        .and_then(|params| params.get(1).cloned())
        .map(|value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transaction decodes from its call — and a BATCH decodes from its
    /// first leg, which is the same thing the phone's sheet shows.
    #[test]
    fn the_decoder_is_handed_the_call_and_not_the_envelope() {
        let single = r#"[{"from":"0xaaa","to":"0xbbb","data":"0xabcd","value":"0x1"}]"#;
        assert_eq!(
            first_call(single),
            Some((
                Some("0xbbb".to_owned()),
                Some("0xabcd".to_owned()),
                Some("0x1".to_owned())
            ))
        );

        let batch = r#"[{"calls":[{"to":"0x1","data":"0xdead"},{"to":"0x2"}]}]"#;
        let (to, data, value) =
            first_call(batch).unwrap_or_else(|| unreachable!("a batch has a first leg"));
        assert_eq!(to.as_deref(), Some("0x1"));
        assert_eq!(data.as_deref(), Some("0xdead"));
        assert_eq!(value, None, "an absent value stays absent, never a zero");
    }

    /// `eth_signTypedData_v4` is `[address, json]`.
    ///
    /// The document is the SECOND parameter. Reading the first hands the
    /// decoder an address where it expects a typed-data payload, and the
    /// whole ladder falls to its blind rung for every typed request — a
    /// degradation nobody would see as a bug, only as "clear signing never
    /// works here".
    #[test]
    fn typed_data_comes_from_the_second_parameter() {
        let params = r#"["0xaaa","{\"primaryType\":\"Permit\"}"]"#;
        assert_eq!(typed_data_of(params), r#"{"primaryType":"Permit"}"#);

        // Some sites pass the document as an object rather than a string.
        let object = r#"["0xaaa",{"primaryType":"Permit"}]"#;
        assert_eq!(typed_data_of(object), r#"{"primaryType":"Permit"}"#);

        assert_eq!(typed_data_of("[]"), "", "nothing to decode is not a panic");
    }
}
