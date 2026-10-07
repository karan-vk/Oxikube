//! `#[gpui::test]` suite for `TerminalElement` (E09-S05): layout and resize, the shaped-row cache,
//! links (cmd/ctrl-click dispatches `terminal::OpenLink`) and the mouse, on the deterministic
//! runtime with `FakeTerminalBackend` and GPUI's test text system (cells 0.6 x font size wide,
//! rows 1.3 x font size tall).

mod cache;
mod layout;
mod links;
mod mouse;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, IntoElement, Modifiers, ParentElement as _,
    Pixels, Point, Render, Styled as _, TestAppContext, Window, div, point, px, size,
};
use oxikube_domain::command::Command;
use oxikube_ports::TerminalSize;
use oxikube_runtime::FRAME_INTERVAL;
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
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            TerminalElement::new(&self.terminal, &self.state, &self.focus)
                .font(self.font.clone())
                .dispatcher(self.recorder.clone())
                .path_links(self.paths.clone()),
        )
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
    cx.update(oxikube_runtime::init_deterministic);
    let backend = FakeTerminalBackend::silent();
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

    fn commands(&self) -> Vec<Command> {
        self.recorder.0.borrow().clone()
    }
}

/// The platform modifier (cmd on macOS, ctrl elsewhere).
fn secondary() -> Modifiers {
    Modifiers::secondary_key()
}
