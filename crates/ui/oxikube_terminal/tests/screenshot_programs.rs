//! Screenshots of recorded full-screen programs (E09-S13): the bytes `vim` (syntax colours, a
//! line-number gutter, `~` filler), `less` (bold and coloured log lines, an inverse prompt) and
//! `tmux` (two panes, a box-drawing border, bold bars in colour, a wide glyph, a coloured status
//! line) wrote to an 80x24 pty, painted by `TerminalElement` in the dark theme.
//!
//! The byte streams are `tests/captures/*.vt` (see `tests/captures/record.py`), parsed in
//! `tests/captures.rs` cell by cell; this file proves they also paint: colours, bold, inverse
//! video and wide glyphs end up as pixels. The element's own features (16 ANSI colours, the
//! 256-colour ramp, every SGR attribute, cursors, selection) are `tests/screenshot.rs`.
//!
//! `harness = false` (the macOS text system lives on the main thread) and needs a GPU device, so
//! it only builds with `--features screenshot` and runs in the nightly job on macOS:
//! `cargo test -p oxikube_terminal --features screenshot --test screenshot_programs`.
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`; a failing run leaves `*.actual.png` and
//! `*.diff.png` next to the golden, which the nightly job uploads.

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px, size,
};
use oxikube_ports::TerminalSize;
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_terminal::{TerminalElement, TerminalElementState, TerminalFont, TerminalState};
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_testkit::gpui_test::{GoldenCase, ScreenshotApp, run_golden_cases};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_theme::{ActiveTheme, Appearance, ThemeTokens};

/// 80 columns x 24 rows at 13 pt with a 1.3 line height, plus a little margin.
const PROGRAM: (u32, u32) = (720, 420);

/// The pinned font: the platform's monospace family, as `tests/screenshot.rs` does.
fn font() -> TerminalFont {
    TerminalFont {
        family: TerminalFont::platform_family().into(),
        size: px(13.),
        line_height: 1.3,
    }
}

/// One terminal showing the screen the program left.
struct Screen {
    terminal: gpui::Entity<TerminalState>,
    state: TerminalElementState,
    focus: gpui::FocusHandle,
}

impl Render for Screen {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(TerminalElement::new(&self.terminal, &self.state, &self.focus).font(font()))
    }
}

/// An 80x24 terminal that has been fed the recording `name`.
fn program(name: &str) -> Result<RgbaImage> {
    let path = format!("{}/tests/captures/{name}.vt", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path)?;
    let mut app = ScreenshotApp::new();
    let backend = FakeTerminalBackend::silent();
    let boxed = Box::new(backend.clone());
    let window: AnyWindowHandle = app.open_window(
        size(px(PROGRAM.0 as f32), px(PROGRAM.1 as f32)),
        |window, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_ui::init(cx);
            pin_dark_theme(cx);
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let terminal = cx.new(|cx| TerminalState::new(boxed, TerminalSize::new(80, 24), cx));
            cx.new(|_| Screen {
                terminal,
                state: TerminalElementState::new(),
                focus,
            })
        },
    )?;
    backend.output(bytes);
    app.run_until_parked();
    app.advance_clock(FRAME_INTERVAL);
    let _ = app.capture(window)?;
    app.advance_clock(FRAME_INTERVAL);
    app.capture(window)
}

/// Pins the theme: `oxikube_theme::init` would follow the system appearance.
fn pin_dark_theme(cx: &mut App) {
    cx.set_global(ActiveTheme(Arc::new(
        ThemeTokens::fallback(Appearance::Dark).clone(),
    )));
}

fn vim() -> Result<RgbaImage> {
    program("vim")
}

fn less() -> Result<RgbaImage> {
    program("less")
}

fn tmux() -> Result<RgbaImage> {
    program("tmux")
}

fn main() -> ExitCode {
    run_golden_cases(
        env!("CARGO_MANIFEST_DIR"),
        &[
            GoldenCase {
                name: "terminal_vim",
                size: PROGRAM,
                render: vim,
            },
            GoldenCase {
                name: "terminal_less",
                size: PROGRAM,
                render: less,
            },
            GoldenCase {
                name: "terminal_tmux",
                size: PROGRAM,
                render: tmux,
            },
        ],
    )
}
