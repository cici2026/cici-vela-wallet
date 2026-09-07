//! The dApp browser's engine — one system webview, living in one column.
//!
//! ## Why a native subview is fine here
//!
//! wry attaches the platform webview (WKWebView on macOS, WebView2 on Windows)
//! as a CHILD of the window gpui already draws, over `raw-window-handle` 0.6 —
//! the one version gpui and wry both pin, so the handle is literally the same
//! type on both sides. What that buys is real: the same engine the person's
//! other browser uses, no second rendering stack, no 200MB of CEF.
//!
//! What it costs is that the subview composites ABOVE everything gpui paints,
//! and that cost lands in exactly two places rather than everywhere:
//!
//! - **Leaving the browser.** A native subview does not disappear because a
//!   gpui route changed. Nothing hides it but this module, so [`hide`] is
//!   called on every frame that is not drawing the browser column. Forget it
//!   and the webview floats over the wallet.
//! - **Centred overlays.** A dialog gpui draws in the middle of the window
//!   (the scanner, `settings_dialog_overlay`) would be painted UNDER the
//!   browser. Those are the ones that must move the webview out of the way.
//!
//! The signing panel is NOT one of them, and that is a layout fact rather than
//! luck: on desktop it is a third COLUMN (`PanelId::Signing` →
//! `panel_scaffold`, the same scaffold Receive and the asset detail use), so it
//! sits beside the browser and shrinks it. The phone's clear-signing drawings
//! show a sheet over the page because a phone has one column to work with.
//!
//! ## Why the origin comes from here and not from the page
//!
//! The extension's content script (spec 027) exists to carry messages plus
//! "exactly two facts the page cannot forge: WHICH tab a request came from,
//! and WHICH origin sent it". This module is that boundary for the desktop,
//! and keeps the same rule: the origin is read from the WEBVIEW's current URL
//! on this side, and a page that puts an `origin` in its own envelope is
//! ignored.

use std::cell::RefCell;

use gpui::{Bounds, Pixels, Window};

/// The provider, verbatim from the extension (spec 027). Injected into every
/// page rather than reimplemented, because a second EIP-1193 implementation is
/// a second set of bugs — and this one has an extension's worth of use behind
/// it.
const INPAGE_JS: &str = include_str!("../../../app-web/vela-wallet/extension/inpage.js");

/// The content script's half, in eleven lines.
///
/// The extension puts `inpage.js` in the MAIN world and talks to it from an
/// isolated one over `window.postMessage`; wry has no isolated world, so this
/// forwards the same envelopes to the host over `window.ipc` and delivers
/// answers back the way the provider already expects them. The provider itself
/// is untouched — it cannot tell the difference, which is the point.
const BRIDGE_JS: &str = r#"
(() => {
  const CHANNEL = 'vela-1193';
  window.addEventListener('message', (ev) => {
    if (ev.source !== window) return;
    const d = ev.data;
    if (!d || d.ch !== CHANNEL || d.dir !== 'req') return;
    // The host adds the origin. Anything this envelope claims about who it is
    // would be the page describing itself.
    window.ipc.postMessage(JSON.stringify({ id: d.id, method: d.method, params: d.params }));
  });
  window.__velaDeliver = (json) => {
    const m = JSON.parse(json);
    window.postMessage({ ch: CHANNEL, ...m }, window.location.origin);
  };
})();
"#;

/// The one browser. A `WebView` is neither `Send` nor `Sync` and belongs to the
/// window it was built as a child of, so it lives on the main thread with the
/// window rather than in a gpui global.
struct Browser {
    view: wry::WebView,
    /// What the webview was last told, so a frame that changed nothing does
    /// not cross the platform boundary sixty times a second.
    bounds: Option<Bounds<Pixels>>,
    visible: bool,
}

thread_local! {
    static BROWSER: RefCell<Option<Browser>> = const { RefCell::new(None) };
}

/// Draw the browser at `bounds`, building it on first call.
///
/// Called from the paint pass of the element that OWNS that rectangle, so the
/// webview follows the column through window resizes and through a third
/// column opening beside it — the signing panel included.
pub fn place(bounds: Bounds<Pixels>, window: &Window, home: &str) {
    BROWSER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = build(window, home).map(|view| Browser {
                view,
                bounds: None,
                visible: false,
            });
        }
        let Some(browser) = slot.as_mut() else {
            return;
        };
        if browser.bounds != Some(bounds) {
            let _ = browser.view.set_bounds(wry::Rect {
                position: wry::dpi::LogicalPosition::new(
                    f64::from(bounds.origin.x),
                    f64::from(bounds.origin.y),
                )
                .into(),
                size: wry::dpi::LogicalSize::new(
                    f64::from(bounds.size.width),
                    f64::from(bounds.size.height),
                )
                .into(),
            });
            browser.bounds = Some(bounds);
        }
        if !browser.visible {
            let _ = browser.view.set_visible(true);
            browser.visible = true;
        }
    });
}

/// Take the browser off the screen.
///
/// Every frame that is not the browser column calls this, because a native
/// subview outlives the gpui route that put it there. It is idempotent and
/// costs nothing when already hidden — a render path may call it sixty times a
/// second and does.
pub fn hide() {
    BROWSER.with(|slot| {
        if let Some(browser) = slot.borrow_mut().as_mut()
            && browser.visible
        {
            let _ = browser.view.set_visible(false);
            browser.visible = false;
        }
    });
}

/// Go to a URL the person typed or a link they picked.
pub fn navigate(url: &str) {
    with_view(|view| {
        let _ = view.load_url(url);
    });
}

/// Back and forward. wry has no native pair, so this is the page's own
/// history — which is the same history the buttons in any browser drive.
pub fn back() {
    with_view(|view| {
        let _ = view.evaluate_script("history.back()");
    });
}

pub fn forward() {
    with_view(|view| {
        let _ = view.evaluate_script("history.forward()");
    });
}

pub fn reload() {
    with_view(|view| {
        let _ = view.reload();
    });
}

/// The host the toolbar shows.
///
/// From the WEBVIEW, so the lock and the name beside it describe the page that
/// is actually loaded — a label fed from anywhere else is a claim about an
/// origin, which is the one thing a browser chrome must never get wrong.
/// `None` before the first page, and the caller keeps drawing what the mock
/// draws rather than an empty bar.
#[must_use]
pub fn host() -> Option<String> {
    BROWSER.with(|slot| {
        let url = slot.borrow().as_ref()?.view.url().ok()?;
        let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(&url);
        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        (!host.is_empty()).then(|| host.to_owned())
    })
}

fn with_view(act: impl FnOnce(&wry::WebView)) {
    BROWSER.with(|slot| {
        if let Some(browser) = slot.borrow().as_ref() {
            act(&browser.view);
        }
    });
}

fn build(window: &Window, home: &str) -> Option<wry::WebView> {
    let built = wry::WebViewBuilder::new()
        .with_initialization_script(INPAGE_JS)
        .with_initialization_script(BRIDGE_JS)
        .with_ipc_handler(on_request)
        .with_url(home)
        .build_as_child(window);
    match built {
        Ok(view) => {
            // Built hidden: `place` turns it on in the same frame, and a
            // webview that flashed into the wallet before its first layout
            // would be visible for exactly one frame in the wrong place.
            let _ = view.set_visible(false);
            Some(view)
        }
        Err(error) => {
            eprintln!("[vela-wallet] browser: {error}");
            None
        }
    }
}

/// One EIP-1193 request from a page.
///
/// Not answered yet — `dapp_session`, `dapp_permissions` and `browser_history`
/// are the next cut. What matters is that it ANSWERS: a provider promise that
/// never settles is the worst outcome this transport can produce (spec 027
/// D37), so an unwired desktop refuses with 4900 rather than leaving a dApp
/// spinning forever.
fn on_request(request: wry::http::Request<String>) {
    let body = request.body();
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return;
    };
    let (Some(id), Some(method)) = (
        parsed.get("id").and_then(|v| v.as_str()),
        parsed.get("method").and_then(|v| v.as_str()),
    ) else {
        return;
    };
    // The ORIGIN is read here, from the webview, never from the envelope.
    let origin = BROWSER.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|browser| browser.view.url().ok())
    });
    println!(
        "[vela-wallet] browser rpc: {method} from {}",
        origin.as_deref().unwrap_or("<unknown origin>")
    );
    let answer = serde_json::json!({
        "dir": "res",
        "id": id,
        "error": { "code": 4900, "message": "Vela's desktop browser cannot answer requests yet" },
    });
    deliver(&answer.to_string());
}

/// Answer a request the signing panel decided.
///
/// The envelope the provider is waiting on: its own `id`, and either a result
/// or an error. The CODE comes from the core — 4001 for a decline, 4900 for
/// stuck-but-submitted — because a wallet that picked its own code here could
/// report a refusal as a failure, and a dApp treats those differently.
#[allow(
    dead_code,
    reason = "called by the signing host once phase 19 opens one"
)]
pub fn respond(id: &str, payload: &vela_core::app::sign_request::SignResponsePayload) {
    use vela_core::app::sign_request::SignResponsePayload;
    let answer = match payload {
        SignResponsePayload::Ok { result } => serde_json::json!({
            "dir": "res",
            "id": id,
            "result": result,
        }),
        SignResponsePayload::Err { code, message, .. } => serde_json::json!({
            "dir": "res",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    };
    deliver(&answer.to_string());
}

fn deliver(json: &str) {
    with_view(|view| {
        let script = format!(
            "window.__velaDeliver({})",
            serde_json::to_string(json).unwrap_or_else(|_| "\"{}\"".to_owned())
        );
        let _ = view.evaluate_script(&script);
    });
}
