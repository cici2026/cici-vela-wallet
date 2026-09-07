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
/// The provider's own constants — `inpage.js` imports these.
const PROTOCOL_JS: &str = include_str!("../../../app-web/vela-wallet/extension/lib/protocol.js");

/// The provider as ONE classic script.
///
/// `inpage.js` is an ES module: it opens with
/// `import { CHANNEL, … } from './lib/protocol.js'`. An initialization script
/// is classic, so injecting it verbatim is a syntax error — the file never
/// runs, `window.ethereum` never appears, and **every request silently never
/// happens**. That is exactly how it failed the first time this was wired,
/// and nothing said so: no error reached the host, the page simply had no
/// wallet in it.
///
/// Concatenating is the CSP-proof fix. Serving the module over a custom
/// protocol and pulling it in with a dynamic `import()` would keep both files
/// untouched, but a dApp with a strict Content-Security-Policy can refuse
/// that, and a provider that works on some sites and not others is worse than
/// one that works everywhere.
///
/// Both files stay byte-identical on disk; the two module keywords are removed
/// HERE, and the test below fails if either file grows another one.
fn provider_script() -> String {
    let constants: String = PROTOCOL_JS
        .lines()
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    let provider: String = INPAGE_JS
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n");
    // One scope, so the constants reach the provider and nothing on the page.
    format!("(() => {{\n{constants}\n{provider}\n}})();")
}

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

/// One EIP-1193 request, with the two facts the page cannot forge attached by
/// this side of the boundary.
pub struct Incoming {
    pub id: String,
    pub method: String,
    pub params_json: String,
    /// From the WEBVIEW's own URL, never from the envelope.
    pub origin: String,
}

/// Where a request goes once it has an origin.
///
/// Installed by the page, because only the page knows which window and which
/// column should answer. Not `Send`: it runs on the main thread, where the
/// webview's callback already is.
type RequestSink = Box<dyn Fn(Incoming)>;

thread_local! {
    static SINK: RefCell<Option<RequestSink>> = const { RefCell::new(None) };
}

/// Hand requests to the page. Called once, when the page builds the browser.
pub fn on_request_to(sink: RequestSink) {
    SINK.with(|slot| *slot.borrow_mut() = Some(sink));
}

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
        .with_initialization_script(provider_script())
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
/// The ORIGIN is read here, from the webview, and the envelope's own idea of
/// who it is is ignored — the rule the extension's content script exists to
/// enforce, and the reason this function and not the page attaches it.
///
/// A request with no sink installed is REFUSED rather than dropped: a
/// provider promise that never settles is the worst outcome this transport
/// can produce (spec 027 D37), and 4900 keeps "we could not answer" distinct
/// from the 4001 that would claim the person declined.
fn on_request(request: wry::http::Request<String>) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(request.body()) else {
        return;
    };
    let (Some(id), Some(method)) = (
        parsed.get("id").and_then(|v| v.as_str()),
        parsed.get("method").and_then(|v| v.as_str()),
    ) else {
        return;
    };
    let origin = BROWSER
        .with(|slot| slot.borrow().as_ref().and_then(|b| b.view.url().ok()))
        .unwrap_or_default();
    let incoming = Incoming {
        id: id.to_owned(),
        method: method.to_owned(),
        params_json: parsed
            .get("params")
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| "[]".to_owned()),
        origin,
    };
    eprintln!(
        "[vela-wallet] browser rpc: {} from {}",
        incoming.method, incoming.origin
    );
    let delivered = SINK.with(|slot| {
        slot.borrow().as_ref().map(|sink| {
            sink(incoming);
        })
    });
    if delivered.is_none() {
        let answer = serde_json::json!({
            "dir": "res",
            "id": id,
            "error": { "code": 4900, "message": "Vela cannot answer this request yet" },
        });
        deliver(&answer.to_string());
    }
}

/// Answer a request the signing panel decided.
///
/// The envelope the provider is waiting on: its own `id`, and either a result
/// or an error. The CODE is the core's — 4001 for a decline, 4900 for
/// stuck-but-submitted — because a shell that picked its own could report a
/// refusal as a failure, and a dApp treats those differently.
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

/// A request this wallet cannot answer yet.
///
/// 4900 and not 4001: the person did not decline, and a dApp that reads a
/// decline where there was none will tell them they refused something they
/// never saw.
pub fn refuse_unsupported(id: &str, method: &str) {
    let answer = serde_json::json!({
        "dir": "res",
        "id": id,
        "error": {
            "code": 4900,
            "message": format!("Vela's desktop cannot answer {method} yet"),
        },
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The provider must be a CLASSIC script by the time it is injected.
    ///
    /// `inpage.js` is an ES module and an initialization script is not, so
    /// injecting it verbatim is a syntax error — the file never runs,
    /// `window.ethereum` never appears, and every request silently never
    /// happens. Nothing reports that: no error reaches the host, the page
    /// simply has no wallet in it. It is how this failed the first time, and
    /// this test is what makes it fail loudly the next time either file grows
    /// another module keyword.
    #[test]
    fn the_injected_provider_carries_no_module_syntax() {
        let script = provider_script();
        for (n, line) in script.lines().enumerate() {
            let line = line.trim_start();
            assert!(
                !line.starts_with("import ") && !line.starts_with("export "),
                "line {} is still module syntax: {line}",
                n + 1
            );
        }
    }

    /// …and that it is still the real provider, not an empty scope. A
    /// concatenation that silently produced nothing would pass the test above
    /// perfectly.
    #[test]
    fn the_injected_provider_is_the_real_one() {
        let script = provider_script();
        assert!(
            script.contains("'vela-1193'"),
            "the channel constant came across from protocol.js"
        );
        assert!(
            script.contains("eip6963:announceProvider"),
            "and the announcement came across from inpage.js"
        );
        assert!(
            script.len() > 10_000,
            "both files, not one: {}",
            script.len()
        );
    }
}
