//! `#[gpui::test]` suite for `TerminalElement` (E09-S05): layout and resize, the shaped-row cache,
//! links (cmd/ctrl-click dispatches `terminal::OpenLink`) and the mouse, on the deterministic
//! runtime with `FakeTerminalBackend` and GPUI's test text system (cells 0.6 x font size wide,
//! rows 1.3 x font size tall). E09-S06 adds the keyboard, IME, clipboard and mouse reporting.

mod cache;
mod clipboard;
mod dialog;
mod ime;
mod keymap_shadowing;
mod keys;
mod layout;
mod links;
mod mouse;
mod report;
mod settings;
mod terminal_keys;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, Modifiers,
    ParentElement as _, Pixels, Point, Render, Styled as _, TestAppContext, Window, div, point, px,
    size,
};
use oxikube_domain::command::Command;
use oxikube_ports::TerminalSize;
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_terminal::input::PasteConfirm;
use oxikube_terminal::{
    PathLinks, TerminalElement, TerminalElementState, TerminalFont, TerminalState,
};
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_testkit::gpui_test::{TestApp, TestWindow};
use oxikube_workspace::CommandDispatcher;

/// Font size of the tests: 6 px cells, 13 px rows.
const FONT_SIZE: f32 = 10.;
const CELL: f32 = 6.;
const ROW: f32 = 13.;

/// Records what the element dispatches.
#[derive(Default)]
struct Recorder(RefCell<Vec<Command>>);

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

/// A view that is nothing but the element.
struct Host {
    terminal: Entity<TerminalState>,
    state: TerminalElementState,
    focus: FocusHandle,
    font: TerminalFont,
    recorder: Rc<Recorder>,
    paths: PathLinks,
    confirm: Option<Rc<dyn PasteConfirm>>,
    /// Draw with the `terminal` font settings instead of `font`.
    themed: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut element = TerminalElement::new(&self.terminal, &self.state, &self.focus)
            .dispatcher(self.recorder.clone())
            .path_links(self.paths.clone());
        if !self.themed {
            element = element.font(self.font.clone());
        }
        if let Some(confirm) = &self.confirm {
            element = element.paste_confirm(confirm.clone());
        }
        // Under the workspace's key context, as in the app, so its bindings apply around the terminal.
        div().key_context("Workspace").size_full().child(element)
    }
}

struct Harness {
    window: TestWindow<Host>,
    backend: FakeTerminalBackend,
    terminal: Entity<TerminalState>,
    state: TerminalElementState,
    recorder: Rc<Recorder>,
}

/// A `width` x `height` px window showing a terminal (started at 80 x 24) at `font_size`.
fn harness(cx: &mut TestAppContext, width: f32, height: f32, font_size: f32) -> Harness {
    harness_with(cx, width, height, font_size, None)
}

/// [`harness`] with a paste confirmation hook.
fn harness_with(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    font_size: f32,
    confirm: Option<Rc<dyn PasteConfirm>>,
) -> Harness {
    harness_on(
        cx,
        width,
        height,
        font_size,
        confirm,
        FakeTerminalBackend::silent(),
    )
}

/// [`harness_with`] over a given backend (an echoing one, for input-to-pixel).
fn harness_on(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    font_size: f32,
    confirm: Option<Rc<dyn PasteConfirm>>,
    backend: FakeTerminalBackend,
) -> Harness {
    build(cx, width, height, font_size, confirm, backend, false)
}

/// A window whose element draws with the `terminal` font settings (themed), like the app's.
fn harness_themed(cx: &mut TestAppContext, width: f32, height: f32) -> Harness {
    build(
        cx,
        width,
        height,
        FONT_SIZE,
        None,
        FakeTerminalBackend::silent(),
        true,
    )
}

fn build(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    font_size: f32,
    confirm: Option<Rc<dyn PasteConfirm>>,
    backend: FakeTerminalBackend,
    themed: bool,
) -> Harness {
    cx.update(oxikube_runtime::init_deterministic);
    let boxed = Box::new(backend.clone());
    let terminal = cx.new(|cx| TerminalState::new(boxed, TerminalSize::new(80, 24), cx));
    let state = TerminalElementState::new();
    let recorder = Rc::new(Recorder::default());
    let mut app = TestApp::new(cx);
    let (t, s, r) = (terminal.clone(), state.clone(), recorder.clone());
    let mut window = app.open_window(move |window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Host {
            terminal: t,
            state: s,
            focus,
            font: TerminalFont {
                family: "Menlo".into(),
                size: px(font_size),
                line_height: 1.3,
            },
            recorder: r,
            paths: PathLinks::Local {
                base: Some(PathBuf::from("/work")),
            },
            confirm,
            themed,
        }
    });
    window.simulate_resize(size(px(width), px(height)));
    window.draw_frame();
    Harness {
        window,
        backend,
        terminal,
        state,
        recorder,
    }
}

impl Harness {
    /// Process output, then one frame: the pump runs, the coalesced notify fires, the window draws.
    fn output(&mut self, bytes: &str) {
        self.backend.output(bytes.to_owned());
        self.frame();
    }

    fn frame(&mut self) {
        self.window.run_until_parked();
        self.window.executor().advance_clock(FRAME_INTERVAL);
        self.window.run_until_parked();
        self.window.draw_frame();
    }

    fn grid(&mut self) -> (u16, u16) {
        let size = self
            .terminal
            .read_with(&mut *self.window, |terminal, _| terminal.size());
        (size.width, size.height)
    }

    /// The middle of viewport cell `row`, `column`.
    fn at(row: usize, column: usize) -> Point<Pixels> {
        point(
            px(CELL * column as f32 + CELL / 2.),
            px(ROW * row as f32 + ROW / 2.),
        )
    }

    /// Everything the process received so far, after letting the writer task run.
    fn written(&mut self) -> Vec<u8> {
        self.window.run_until_parked();
        self.backend.written()
    }

    fn commands(&self) -> Vec<Command> {
        self.recorder.0.borrow().clone()
    }
}

/// Installs a settings store with `user` (a JSON object of the user's settings) on top of the
/// shipped defaults, so the terminal input settings can be changed.
fn configure(cx: &mut TestAppContext, user: &str) {
    use gpui::UpdateGlobal as _;
    cx.update(|cx| {
        if !cx.has_global::<oxikube_settings::SettingsStore>() {
            let store = oxikube_settings::SettingsStore::new(oxikube_assets::default_settings())
                .expect("the shipped defaults parse");
            cx.set_global(store);
            oxikube_terminal::init(cx);
        }
        oxikube_settings::SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(user).expect("valid settings")
        });
    });
}

/// The platform modifier (cmd on macOS, ctrl elsewhere).
fn secondary() -> Modifiers {
    Modifiers::secondary_key()
}
