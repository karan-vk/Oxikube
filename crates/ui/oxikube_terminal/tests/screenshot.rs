//! Screenshots of `TerminalElement` (E09-S05), rendered through `Window::render_to_image`.
//!
//! - `terminal_dark`, `terminal_light`: one terminal fed the fixture below in the dark and the
//!   light theme: the 16 ANSI colours as foreground and background, the 256-colour cube and grey
//!   ramp, truecolour, bold / italic / dim / inverse / hidden, every underline style and
//!   strikethrough, CJK and emoji (wide glyphs), combining marks, a URL hovered with the platform
//!   modifier (underlined), a selection over two rows and the focused block cursor.
//! - `terminal_cursors`: four small terminals: block, beam and underline cursors (focused) and
//!   the hollow block of an unfocused terminal.
//!
//! - `terminal_preedit` (E09-S06): an input method composing four Japanese syllables at the shell
//!   prompt: the marked text is underlined at the cursor, over the cells after it.
//! - `terminal_tabs` (E09-S07): terminal tabs in a workspace: one in the centre pane (its title
//!   set by the process, the dirty dot of a running process), one in the bottom dock beside the
//!   terminal panel, carrying a read-only cluster's mark, and the exit line of an ended one.
//!
//! - `terminal_settings` (E09-S11): the `terminal` settings drive the element: an 18 pt font with
//!   a 1.5 line height and a steady underline cursor, no font passed to the element.
//! - `terminal_search` (E09-S11): the matches of a search painted over the cells, the current one
//!   in its own colour.
//!
//! - `terminal_banner_gone` (E06-U558): a pod terminal whose pod no longer exists: one plain
//!   sentence, the server's words open behind Details, and Close instead of Reconnect.
//!
//! The byte stream is in this file, so the picture only changes when the element (or the font
//! the platform ships) does. `harness = false`: on macOS the platform text system can only be
//! created on the process main thread. Needs a GPU device (Metal, or Vulkan such as Mesa lavapipe
//! on Linux), so it only builds with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_terminal --features screenshot --test screenshot`.
//!
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EntityInputHandler as _, FocusHandle,
    IntoElement, Modifiers, MouseMoveEvent, ParentElement as _, PlatformInput, Render, Styled as _,
    Window, div, px, size,
};
use oxikube_ports::TerminalSize;
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_terminal::element::CellMetrics;
use oxikube_terminal::grid::{GridPoint, SelectionKind, SelectionSide};
use oxikube_terminal::{TerminalElement, TerminalElementState, TerminalFont, TerminalState};
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_theme::{ActiveTheme, Appearance, ThemeTokens};

const FULL: (u32, u32) = (720, 300);
const CURSORS: (u32, u32) = (480, 60);

/// The fixture: one feature per line.
const FIXTURE: &str = concat!(
    "\x1b[30m black\x1b[31m red\x1b[32m green\x1b[33m yellow\x1b[34m blue\x1b[35m magenta\x1b[36m cyan\x1b[37m white\x1b[0m\r\n",
    "\x1b[90m black\x1b[91m red\x1b[92m green\x1b[93m yellow\x1b[94m blue\x1b[95m magenta\x1b[96m cyan\x1b[97m white\x1b[0m\r\n",
    "\x1b[40m  \x1b[41m  \x1b[42m  \x1b[43m  \x1b[44m  \x1b[45m  \x1b[46m  \x1b[47m  ",
    "\x1b[100m  \x1b[101m  \x1b[102m  \x1b[103m  \x1b[104m  \x1b[105m  \x1b[106m  \x1b[107m  \x1b[0m 16 colours\r\n",
    "\x1b[48;5;16m \x1b[48;5;52m \x1b[48;5;88m \x1b[48;5;124m \x1b[48;5;160m \x1b[48;5;196m \x1b[48;5;202m \x1b[48;5;208m \x1b[48;5;214m \x1b[48;5;220m \x1b[48;5;226m \x1b[48;5;46m \x1b[48;5;51m \x1b[48;5;21m \x1b[48;5;93m ",
    "\x1b[48;5;232m \x1b[48;5;236m \x1b[48;5;240m \x1b[48;5;244m \x1b[48;5;248m \x1b[48;5;252m \x1b[48;5;255m \x1b[0m ",
    "\x1b[38;2;255;100;0mtrue\x1b[38;2;0;160;255mcolour\x1b[0m\r\n",
    "\x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[1;3mboth\x1b[0m \x1b[2mdim\x1b[0m \x1b[7minverse\x1b[0m [\x1b[8mhidden\x1b[0m]\r\n",
    "\x1b[4munderline\x1b[0m \x1b[4:2mdouble\x1b[0m \x1b[4:3mcurly\x1b[0m \x1b[4:4mdotted\x1b[0m \x1b[4:5mdashed\x1b[0m \x1b[9mstrike\x1b[0m\r\n",
    "wide: \u{4f60}\u{597d}\u{4e16}\u{754c} \u{1f600}\u{1f680} mixed a\u{4e2d}b  combining: e\u{301} n\u{303}\r\n",
    "link: https://kubernetes.io/docs  path: src/main.rs:12:3\r\n",
    "selected text spans\r\n",
    "two rows here\r\n",
    "$ ",
);

/// Pins the theme: `oxikube_theme::init` would follow the system appearance.
fn set_theme(appearance: Appearance, cx: &mut App) {
    cx.set_global(ActiveTheme(Arc::new(
        ThemeTokens::fallback(appearance).clone(),
    )));
}

fn font() -> TerminalFont {
    TerminalFont {
        family: TerminalFont::platform_family().into(),
        size: px(13.),
        line_height: 1.3,
    }
}

/// Terminals side by side, each with its own focus handle (only the first set is focused).
struct Panes {
    panes: Vec<(Entity<TerminalState>, TerminalElementState, FocusHandle)>,
}

impl Render for Panes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_row()
            .gap(px(8.))
            .children(self.panes.iter().map(|(terminal, state, focus)| {
                div()
                    .flex_1()
                    .h_full()
                    .child(TerminalElement::new(terminal, state, focus).font(font()))
            }))
    }
}

/// Opens a window of `window_size` with one terminal per entry of `feeds` (all but the last
/// focused when `hollow_last`), feeds them and settles.
fn render(
    appearance: Appearance,
    window_size: (u32, u32),
    feeds: &[&str],
    hollow_last: bool,
    select: bool,
    compose: Option<&str>,
) -> Result<RgbaImage> {
    let mut app = ScreenshotApp::new();
    let backends: Vec<FakeTerminalBackend> = feeds
        .iter()
        .map(|_| FakeTerminalBackend::silent())
        .collect();
    let mut terminals = Vec::new();
    let window: AnyWindowHandle = app.open_window(
        size(px(window_size.0 as f32), px(window_size.1 as f32)),
        |window, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_ui::init(cx);
            set_theme(appearance, cx);
            let focused = cx.focus_handle();
            window.focus(&focused, cx);
            let panes = backends
                .iter()
                .enumerate()
                .map(|(index, backend)| {
                    let boxed = Box::new(backend.clone());
                    let terminal =
                        cx.new(|cx| TerminalState::new(boxed, TerminalSize::new(80, 24), cx));
                    terminals.push(terminal.clone());
                    let focus = if hollow_last && index + 1 == feeds.len() {
                        cx.focus_handle()
                    } else {
                        focused.clone()
                    };
                    (terminal, TerminalElementState::new(), focus)
                })
                .collect();
            cx.new(|_| Panes { panes })
        },
    )?;
    for (backend, feed) in backends.iter().zip(feeds) {
        backend.output(feed.to_string());
    }
    app.run_until_parked();
    app.advance_clock(FRAME_INTERVAL);
    let _ = app.capture(window)?;
    if select && let Some(terminal) = terminals.first() {
        app.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.start_selection(
                    SelectionKind::Cell,
                    GridPoint::new(8, 9),
                    SelectionSide::Left,
                    cx,
                );
                terminal.update_selection(GridPoint::new(9, 2), SelectionSide::Right, cx);
            })
        });
        hover_link(&mut app, window)?;
    }
    if let (Some(text), Some(terminal)) = (compose, terminals.first()) {
        app.update(|cx| {
            window.update(cx, |_, window, cx| {
                terminal.update(cx, |terminal, cx| {
                    terminal.replace_and_mark_text_in_range(None, text, None, window, cx)
                })
            })
        })?;
    }
    app.advance_clock(FRAME_INTERVAL);
    app.capture(window)
}

/// Moves the pointer over the URL of the fixture's link row with the platform modifier held: the
/// link is underlined.
fn hover_link(app: &mut ScreenshotApp, window: AnyWindowHandle) -> Result<()> {
    app.update(|cx| {
        window.update(cx, |_, window, cx| {
            let metrics = CellMetrics::measure(&font(), window.text_system());
            let position = metrics.cell_origin(gpui::point(px(0.), px(0.)), 7, 12)
                + gpui::point(metrics.cell_width / 2., metrics.line_height / 2.);
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::secondary_key(),
                }),
                cx,
            );
        })
    })
}

fn full_dark() -> Result<RgbaImage> {
    render(Appearance::Dark, FULL, &[FIXTURE], false, true, None)
}

fn full_light() -> Result<RgbaImage> {
    render(Appearance::Light, FULL, &[FIXTURE], false, true, None)
}

fn cursors() -> Result<RgbaImage> {
    render(
        Appearance::Dark,
        CURSORS,
        &[
            "block x",
            "beam \x1b[6 qx\x1b[D",
            "under \x1b[4 qx\x1b[D",
            "hollow x",
        ],
        true,
        false,
        None,
    )
}

fn preedit() -> Result<RgbaImage> {
    render(
        Appearance::Dark,
        CURSORS,
        &["$ echo "],
        false,
        false,
        Some("\u{306b}\u{307b}\u{3093}\u{3054}"),
    )
}

/// The fake launcher of `terminal_tabs`: a silent fake backend per launch.
struct ShotLauncher(std::cell::RefCell<Vec<FakeTerminalBackend>>);

impl oxikube_terminal::view::TerminalLauncher for ShotLauncher {
    fn launch(
        &self,
        _: &oxikube_terminal::view::BackendDescriptor,
        _: TerminalSize,
        _: &mut App,
    ) -> oxikube_terminal::view::Launch {
        let backend = FakeTerminalBackend::silent();
        self.0.borrow_mut().push(backend.clone());
        gpui::Task::ready(Ok(Box::new(backend)))
    }

    fn cluster_mark(
        &self,
        _: &oxikube_domain::ids::ClusterId,
        _: &App,
    ) -> Option<oxikube_workspace::ClusterMark> {
        Some(oxikube_workspace::ClusterMark {
            colour: None,
            read_only: true,
        })
    }
}

const TABS: (u32, u32) = (720, 420);

fn tabs() -> Result<RgbaImage> {
    use oxikube_domain::ids::{ClusterId, ContextName};
    use oxikube_terminal::view::{BackendDescriptor, TerminalServices, TerminalView};
    use oxikube_workspace::{DockPosition, Workspace};

    // The tab icons and the read-only lock are SVGs from the app's assets.
    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    let launcher = std::rc::Rc::new(ShotLauncher(Default::default()));
    let services = TerminalServices::new(launcher.clone());
    let cluster = ClusterId::new("~/.kube/config", &ContextName::new("prod-eu"));
    let window: AnyWindowHandle =
        app.open_window(size(px(TABS.0 as f32), px(TABS.1 as f32)), |window, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_ui::init(cx);
            set_theme(Appearance::Dark, cx);
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            let centre = cx.new(|cx| {
                let shell = BackendDescriptor::local(None).with_shell("/bin/zsh", vec![]);
                TerminalView::new(shell, services.clone(), cx)
            });
            let docked = cx.new(|cx| {
                let shell =
                    BackendDescriptor::local(Some(cluster.clone())).with_shell("bash", vec![]);
                TerminalView::new(shell, services.clone(), cx)
            });
            let ended = cx.new(|cx| {
                let shell = BackendDescriptor::local(None).with_shell("sh", vec![]);
                TerminalView::new(shell, services.clone(), cx)
            });
            oxikube_terminal::view::ensure_terminal_panel(
                &workspace,
                Some(cluster.clone()),
                None,
                window,
                cx,
            );
            workspace.update(cx, |ws, cx| {
                ws.open_item(centre, window, cx);
                ws.open_item_in_split(
                    Box::new(ended),
                    None,
                    oxikube_workspace::SplitDirection::Right,
                    window,
                    cx,
                );
                ws.open_item_in_dock(Box::new(docked), DockPosition::Bottom, false, window, cx);
                let first = ws.panes(cx)[0].id();
                ws.activate_pane(first, window, cx);
            });
            oxikube_ui::root::new_root(workspace, window, cx)
        })?;
    app.run_until_parked();
    let backends = launcher.0.borrow().clone();
    // Launch order: the centre shell, the docked cluster shell, the ended one.
    backends[0].output("\x1b]0;kubectl get pods\x07$ kubectl get pods\r\nNAME    READY   STATUS    RESTARTS   AGE\r\nweb-0   1/1     Running   0          3d\r\n$ ".to_owned());
    backends[1].output("$ helm list -n shop\r\nNAME  NAMESPACE  REVISION  STATUS\r\nshop  shop       7         deployed\r\n$ ".to_owned());
    backends[2].output("$ exit 2\r\n".to_owned());
    backends[2].exit(oxikube_ports::ExitStatus::with_code(2));
    app.run_until_parked();
    app.advance_clock(FRAME_INTERVAL);
    let _ = app.capture(window)?;
    app.advance_clock(FRAME_INTERVAL);
    app.capture(window)
}

/// A launcher whose pod is gone: every launch fails with `NotFound`.
struct GoneLauncher;

impl oxikube_terminal::view::TerminalLauncher for GoneLauncher {
    fn launch(
        &self,
        _: &oxikube_terminal::view::BackendDescriptor,
        _: TerminalSize,
        _: &mut App,
    ) -> oxikube_terminal::view::Launch {
        gpui::Task::ready(Err(oxikube_domain::OxiError::not_found(
            "pods \"web-0\" not found",
        )))
    }
}

const GONE: (u32, u32) = (720, 200);

fn banner_gone() -> Result<RgbaImage> {
    use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
    use oxikube_terminal::view::{BackendDescriptor, TerminalServices, TerminalView};
    use oxikube_workspace::Workspace;

    let mut app = ScreenshotApp::with_assets(Arc::new(oxikube_ui::Assets));
    let services = TerminalServices::new(std::rc::Rc::new(GoneLauncher));
    let cluster = ClusterId::new("~/.kube/config", &ContextName::new("prod-eu"));
    let mut view = None;
    let window: AnyWindowHandle =
        app.open_window(size(px(GONE.0 as f32), px(GONE.1 as f32)), |window, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_ui::init(cx);
            set_theme(Appearance::Dark, cx);
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            let pod = BackendDescriptor::Exec {
                pod: ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "shop", "web-0"),
                container: Some("app".into()),
                command: vec!["/bin/sh".into()],
            };
            let terminal = cx.new(|cx| TerminalView::new(pod, services.clone(), cx));
            view = Some(terminal.clone());
            workspace.update(cx, |ws, cx| ws.open_item(terminal, window, cx));
            oxikube_ui::root::new_root(workspace, window, cx)
        })?;
    app.run_until_parked();
    app.update(|cx| {
        if let Some(view) = &view {
            view.update(cx, |view, cx| view.toggle_banner_details(cx));
        }
    });
    app.run_until_parked();
    app.advance_clock(FRAME_INTERVAL);
    let _ = app.capture(window)?;
    app.advance_clock(FRAME_INTERVAL);
    app.capture(window)
}

/// A terminal drawn with the `terminal` settings (no font given to the element), with the matches
/// of a search over it.
struct Themed {
    terminal: Entity<TerminalState>,
    state: TerminalElementState,
    focus: FocusHandle,
    highlights: Option<oxikube_terminal::element::SearchHighlights>,
}

impl Render for Themed {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut element = TerminalElement::new(&self.terminal, &self.state, &self.focus);
        if let Some(highlights) = &self.highlights {
            element = element.highlights(highlights.clone());
        }
        div().size_full().child(element)
    }
}

const SETTINGS: (u32, u32) = (480, 150);

/// `feed` shown under the settings `user`, with the matches of `search` painted when given.
fn themed(user: &str, feed: &str, search: Option<&str>) -> Result<RgbaImage> {
    use gpui::UpdateGlobal as _;
    let mut app = ScreenshotApp::new();
    let backend = FakeTerminalBackend::silent();
    let boxed = Box::new(backend.clone());
    let mut host = None;
    let mut terminal_handle = None;
    let window: AnyWindowHandle = app.open_window(
        size(px(SETTINGS.0 as f32), px(SETTINGS.1 as f32)),
        |window, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_ui::init(cx);
            set_theme(Appearance::Dark, cx);
            let store = oxikube_settings::SettingsStore::new(oxikube_assets::default_settings())
                .expect("the shipped defaults");
            cx.set_global(store);
            oxikube_terminal::init(cx);
            oxikube_settings::SettingsStore::update_global(cx, |store, _| {
                store.set_user_settings(user).expect("valid settings")
            });
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let terminal = cx.new(|cx| TerminalState::new(boxed, TerminalSize::new(80, 24), cx));
            terminal_handle = Some(terminal.clone());
            let view = cx.new(|_| Themed {
                terminal,
                state: TerminalElementState::new(),
                focus,
                highlights: None,
            });
            host = Some(view.clone());
            view
        },
    )?;
    backend.output(feed.to_owned());
    app.run_until_parked();
    app.advance_clock(FRAME_INTERVAL);
    let _ = app.capture(window)?;
    if let (Some(pattern), Some(terminal), Some(host)) = (search, terminal_handle, host) {
        app.update(|cx| {
            let found = terminal.update(cx, |terminal, cx| terminal.search(pattern, cx));
            cx.spawn(async move |cx| {
                let matches = found.await.expect("a valid pattern");
                host.update(cx, |host, cx| {
                    host.highlights = Some(oxikube_terminal::element::SearchHighlights::sorted(
                        matches,
                        Some(1),
                    ));
                    cx.notify();
                });
            })
            .detach();
        });
        app.run_until_parked();
        app.advance_clock(FRAME_INTERVAL);
    }
    app.advance_clock(FRAME_INTERVAL);
    app.capture(window)
}

fn settings_shot() -> Result<RgbaImage> {
    themed(
        r#"{ "terminal": { "font_size": 18, "line_height": 1.5,
                           "cursor_shape": "underline", "cursor_blink": false } }"#,
        "$ kubectl get pods\r\nNAME    READY   STATUS\r\nweb-0   1/1     Running\r\n$ ls",
        None,
    )
}

fn search_shot() -> Result<RgbaImage> {
    themed(
        "{}",
        "NAME    READY   STATUS\r\nweb-0   1/1     Running\r\nweb-1   1/1     Running\r\ndb-0    0/1     Pending\r\n$ ",
        Some("web-\\d"),
    )
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "terminal_dark",
                size: FULL,
                render: full_dark,
            },
            GoldenCase {
                name: "terminal_light",
                size: FULL,
                render: full_light,
            },
            GoldenCase {
                name: "terminal_cursors",
                size: CURSORS,
                render: cursors,
            },
            GoldenCase {
                name: "terminal_preedit",
                size: CURSORS,
                render: preedit,
            },
            GoldenCase {
                name: "terminal_tabs",
                size: TABS,
                render: tabs,
            },
            GoldenCase {
                name: "terminal_banner_gone",
                size: GONE,
                render: banner_gone,
            },
            GoldenCase {
                name: "terminal_settings",
                size: SETTINGS,
                render: settings_shot,
            },
            GoldenCase {
                name: "terminal_search",
                size: SETTINGS,
                render: search_shot,
            },
        ],
    )
}
