//! Vela Wallet desktop entry — window management only (spec 007 FR-009).

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod ceremony;
mod contacts;
mod core_host;
mod ctap;
mod executor;
mod explore;
mod flows;
mod gallery;
mod hardware;
mod icons;
mod identicon;
mod loc;
mod onboarding;
mod onboarding_flow;
mod outcome;
mod parallel_space;
mod passkey_directory;
mod raster;
mod resident;
mod session;
mod settings;
mod signing;
mod theme;
mod ui;
mod wallet;
mod window_frame;

use gallery::GalleryView;
use gpui::{
    App, AppContext as _, Bounds, Context, Div, IntoElement, KeyBinding, Menu, MenuItem,
    ParentElement as _, QuitMode, Render, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowOptions, actions, div, point, px, size,
};
use onboarding::OnboardingPage;
use theme::{WINDOW_H, WINDOW_W};
use vela_core::app::session::SessionRoute;
use wallet::page::{Identity, WalletPage};

/// The window's one child, and the only thing in this app that decides which
/// screen a person is on.
///
/// **The core decides WHAT is allowed; this decides WHEN to navigate.** It
/// renders `SessionView::allowed_route` and concludes nothing of its own — in
/// particular it does not read the account list and infer a route, because
/// during the storage read there is no answer yet and `Loading` is how the core
/// says so. Rendering onboarding in that gap would flash the welcome screen at
/// somebody who already has a wallet.
struct Root {
    onboarding: Option<gpui::Entity<OnboardingPage>>,
    wallet: Option<gpui::Entity<WalletPage>>,
}

impl Root {
    fn new(cx: &mut Context<Self>) -> Self {
        // Re-render whenever the session changes: the hand-off out of
        // onboarding is a global write, and this is what turns it into a
        // navigation.
        cx.observe_global::<session::SessionState>(|_, cx| cx.notify())
            .detach();
        Self {
            onboarding: None,
            wallet: None,
        }
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = session::view(cx);
        let root: Div = div().size_full();
        let root = match view.allowed_route {
            // Storage unread. Paint the surface and nothing else — a splash
            // that lasts one frame is invisible, and a wrong screen is not.
            SessionRoute::Loading => {
                root.bg(theme::Theme::of(theme::ThemeMode::detect(window)).bg_base)
            }
            SessionRoute::Onboarding => {
                // Dropped on the way out, so a second sign-in starts from a
                // fresh machine rather than resuming a finished one.
                self.wallet = None;
                // And so do the resident machines. Contacts, networks and the
                // chosen currency belong to the ACCOUNT; a resident that
                // outlived a sign-out would show the previous person's address
                // book to the next one.
                resident::drop_all(cx);
                let page = self
                    .onboarding
                    .get_or_insert_with(|| cx.new(|cx| OnboardingPage::new(window, cx)))
                    .clone();
                root.child(page)
            }
            SessionRoute::Wallet => {
                self.onboarding = None;
                let identity = Identity {
                    name: view
                        .accounts
                        .get(view.active_index)
                        .map_or_else(|| "".into(), |row| row.account.name.clone().into()),
                    address: view.address.clone(),
                };
                let page = self
                    .wallet
                    .get_or_insert_with(|| cx.new(|cx| WalletPage::signed_in(identity, window, cx)))
                    .clone();
                root.child(page)
            }
        };
        // The parallel space's marker, over whichever screen is up. It renders
        // whenever the space is active and never behind a build flag alone —
        // a test wallet must never wear the real one's face.
        parallel_space::overlay(root, window)
    }
}

/// Which root the window hosts.
/// `VELA_PAGE=wallet|contacts|explore|settings|gallery`
/// (spec 015 research.md D4, extended by spec 018 research.md D1 and spec 023)
/// — same env-pin family as `VELA_THEME`/`VELA_LANG`; the default remains the
/// onboarding flow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RootPage {
    Onboarding,
    Wallet,
    Contacts,
    /// Spec 022 — the browser, for review without clicking through the wallet.
    Explore,
    Settings,
    Gallery,
}

impl RootPage {
    fn from_env() -> Self {
        match std::env::var("VELA_PAGE").as_deref() {
            Ok("wallet") => Self::Wallet,
            Ok("contacts") => Self::Contacts,
            Ok("explore") => Self::Explore,
            Ok("settings") => Self::Settings,
            Ok("gallery") => Self::Gallery,
            _ => Self::Onboarding,
        }
    }
}

// TODO(i18n): menu labels are English-only until the corpus grows menu keys.
actions!(vela, [Quit, HideApp, HideOthers, ShowAll, ToggleFullScreen]);

/// Open the (only) application window. Called at startup, and again from the
/// reopen handler when the Dock icon is clicked after the window was closed.
fn open_main_window(cx: &mut App) {
    // Two dev galleries coexist: `VELA_GALLERY=1` (spec 014) browses the
    // onboarding-flow state fixtures; `VELA_PAGE=gallery` (spec 015) browses
    // the wallet-home states. Unifying them is a recorded follow-up.
    if gallery::gallery_enabled() {
        open_window_with(cx, |window, cx| cx.new(|cx| GalleryView::new(window, cx)));
        return;
    }
    match RootPage::from_env() {
        // The default: the session's route guard picks the screen.
        RootPage::Onboarding => open_window_with(cx, |_window, cx| cx.new(Root::new)),
        RootPage::Wallet => open_window_with(cx, |window, cx| {
            cx.new(|cx| WalletPage::new(false, window, cx))
        }),
        RootPage::Contacts => open_window_with(cx, |window, cx| {
            cx.new(|cx| WalletPage::contacts(window, cx))
        }),
        RootPage::Explore => open_window_with(cx, |window, cx| {
            cx.new(|cx| WalletPage::explore(window, cx))
        }),
        RootPage::Settings => open_window_with(cx, |window, cx| {
            cx.new(|cx| WalletPage::settings(window, cx))
        }),
        RootPage::Gallery => open_window_with(cx, |window, cx| {
            cx.new(|cx| WalletPage::new(true, window, cx))
        }),
    }
}

fn open_window_with<V: gpui::Render + 'static>(
    cx: &mut App,
    build: impl FnOnce(&mut gpui::Window, &mut App) -> gpui::Entity<V>,
) {
    let bounds = Bounds::centered(None, size(px(WINDOW_W), px(WINDOW_H)), cx);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // The card grid does not reflow below the design size (spec 007
            // edge cases): the design size is the minimum.
            window_min_size: Some(size(px(WINDOW_W), px(WINDOW_H))),
            // Sets the Wayland `xdg_toplevel.app_id` and the X11 `WM_CLASS`.
            // Both are how a desktop shell matches a running window to its
            // installed `.desktop` file, so this string, the file name
            // `packaging/app.getvela.VelaWallet.desktop` and the
            // `StartupWMClass` inside it must stay identical — otherwise
            // GNOME shows the app with a generic icon and the binary name.
            app_id: Some("app.getvela.VelaWallet".into()),
            titlebar: Some(TitlebarOptions {
                title: Some("Vela Wallet".into()),
                // Content owns the full canvas, as in the mocks; only the
                // traffic lights remain, inset to the mock's position.
                appears_transparent: true,
                traffic_light_position: Some(point(px(20.), px(20.))),
            }),
            ..Default::default()
        },
        |window, cx| {
            let view = build(window, cx);
            // Dropping a `WebView` removes it from the window, so the probe
            // has to outlive this closure. A thread local rather than a gpui
            // global because it is neither `Send` nor `Sync` and lives on the
            // main thread by construction — and because a probe should be
            // obviously temporary scaffolding, not a field on the app.
            #[cfg(not(target_os = "linux"))]
            if let Some(webview) = attach_webview_probe(window) {
                WEBVIEW_PROBE.with(|slot| *slot.borrow_mut() = Some(webview));
            }
            view
        },
    )
    .expect("failed to open the main window");
}

#[cfg(not(target_os = "linux"))]
thread_local! {
    static WEBVIEW_PROBE: std::cell::RefCell<Option<wry::WebView>> =
        const { std::cell::RefCell::new(None) };
}

/// The dApp browser's engine, proven rather than argued about (spec 032
/// phase 13).
///
/// `VELA_WEBVIEW=1` attaches a real webview to the window this app already
/// draws, loads a page in it, and lets that page talk back. It answers the one
/// question a table of engines cannot: does a system webview embed inside a
/// gpui window at all.
///
/// The mechanism is `gpui::Window: HasWindowHandle` meeting wry's
/// `build_as_child`, over `raw-window-handle` 0.6 — the one version gpui and
/// wry already agree on. On macOS the result is an `NSView` subview of the
/// window's content view; on Windows a child `HWND`.
///
/// **Composited ABOVE everything gpui draws**, because that is what a native
/// subview is. Any panel this app puts over the browser — the signing sheet
/// above all — has to hide or move the webview rather than draw on top of it,
/// which is what `set_visible` and `set_bounds` are for.
#[cfg(not(target_os = "linux"))]
fn attach_webview_probe(window: &gpui::Window) -> Option<wry::WebView> {
    if std::env::var("VELA_WEBVIEW").as_deref() != Ok("1") {
        return None;
    }
    // A page that reports back, so the probe proves the BRIDGE and not just
    // that pixels arrived: the injected script is where `window.ethereum`
    // would be defined, and the ipc handler is the channel a provider answers
    // over.
    let webview = wry::WebViewBuilder::new()
        .with_bounds(wry::Rect {
            position: wry::dpi::LogicalPosition::new(520.0, 120.0).into(),
            size: wry::dpi::LogicalSize::new(760.0, 640.0).into(),
        })
        .with_initialization_script(
            "window.__vela = { probe: () => window.ipc.postMessage('vela:probe') };
             window.addEventListener('DOMContentLoaded', () => window.__vela.probe());",
        )
        .with_ipc_handler(|request| {
            println!("[vela-wallet] webview ipc: {}", request.body());
        })
        .with_html(
            "<html><body style='font:16px -apple-system;padding:24px'>
             <h2>Vela webview probe</h2><p>If you can read this, a system
             webview is embedded in the gpui window.</p></body></html>",
        )
        .build_as_child(window);
    match webview {
        Ok(webview) => {
            println!("[vela-wallet] webview: attached as a child of the gpui window");
            Some(webview)
        }
        Err(error) => {
            eprintln!("[vela-wallet] webview: {error}");
            None
        }
    }
}

fn main() {
    // LastWindowClosed instead of gpui's macOS default (keep running): a
    // keep-running single-window app must reopen its window from the Dock
    // icon, and that path is dead on macOS 26 — AppKit's TextInputUI panel
    // stays `isVisible` after the last real window closes, the system then
    // reports hasVisibleWindows=YES on a Dock click, and gpui's
    // `should_handle_reopen` swallows the event before any on_reopen callback
    // (README, "Known gpui quirks"). Closing the window therefore quits, the
    // same behaviour gpui already defaults to on Windows and Linux.
    let app = gpui_platform::application().with_quit_mode(QuitMode::LastWindowClosed);

    // Insurance, not the fix: unreachable while LastWindowClosed holds, but
    // if a future gpui or quit-mode change ever leaves the app running with
    // no window, a Dock-icon click should build one rather than nothing.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(cx);
        }
    });

    app.run(|cx: &mut App| {
        // Storage is read before the first window opens, so the route guard has
        // a real answer to give on frame one.
        session::boot(cx);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &HideApp, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.on_action(|_: &ToggleFullScreen, cx| {
            if let Some(window) = cx.active_window() {
                window
                    .update(cx, |_, window, _| window.toggle_fullscreen())
                    .ok();
            }
        });

        // The main menu is not just convention on macOS: with a nil
        // NSApp.mainMenu, the menu-bar/titlebar reveal on a fullscreen Space
        // never engages on secondary displays, so a fullscreened window there
        // shows no titlebar on hover and offers no way back out. gpui installs
        // no menu of its own — Zed always sets one, which is why upstream
        // never trips over this.
        if cfg!(target_os = "macos") {
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-h", HideApp, None),
                KeyBinding::new("alt-cmd-h", HideOthers, None),
                KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
            ]);
            cx.set_menus(vec![
                // The first menu is the application menu; macOS titles it with
                // the bundle name, not this string.
                Menu::new("Vela Wallet").items(vec![
                    MenuItem::action("Hide Vela Wallet", HideApp),
                    MenuItem::action("Hide Others", HideOthers),
                    MenuItem::action("Show All", ShowAll),
                    MenuItem::separator(),
                    MenuItem::action("Quit Vela Wallet", Quit),
                ]),
                Menu::new("View").items(vec![MenuItem::action(
                    "Toggle Full Screen",
                    ToggleFullScreen,
                )]),
            ]);
        }
        open_main_window(cx);

        cx.activate(true);
    });
}
