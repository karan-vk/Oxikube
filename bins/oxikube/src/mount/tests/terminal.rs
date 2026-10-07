//! `terminal::OpenLink` (E09-S05) on the app's bus: a valid link reaches the platform's opener on
//! the UI thread, anything else is refused before it.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn open(app: &mut App, target: &str) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(
        Command::TerminalOpenLink {
            target: target.into(),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

#[gpui::test]
fn a_terminal_link_opens_in_the_browser(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(open(&mut app, "https://kubernetes.io/docs/"));
    assert_eq!(
        app.vcx.opened_url().as_deref(),
        Some("https://kubernetes.io/docs/")
    );
}

#[gpui::test]
fn unsafe_or_missing_links_are_refused(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(!open(&mut app, "javascript:alert(1)"));
    assert!(!open(
        &mut app,
        "/definitely/not/a/file/on/this/machine.rs:1:2"
    ));
    assert!(!open(&mut app, "relative/path.rs"));
    assert_eq!(app.vcx.opened_url(), None, "nothing reached the platform");
}

fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

#[gpui::test]
fn terminal_copy_and_paste_reach_the_window(cx: &mut TestAppContext) {
    // E09-S06: both are queued by the bus handlers, drained by the window's `terminal_input` task
    // and dispatched as the `terminal::Copy` / `terminal::Paste` actions (to the focused terminal;
    // no terminal is mounted in the app yet, so a global listener stands in for it). Without the
    // drain the bus call still succeeds, so the actions are what the test looks at.
    use std::cell::RefCell;
    use std::rc::Rc;

    use oxikube_terminal::input::{Copy, Paste};

    let mut app = App::start(cx, TestPorts::seeded());
    let seen = Rc::new(RefCell::new(Vec::new()));
    app.vcx.update(|_, cx| {
        let log = seen.clone();
        cx.on_action(move |_: &Copy, _| log.borrow_mut().push("copy"));
        let log = seen.clone();
        cx.on_action(move |_: &Paste, _| log.borrow_mut().push("paste"));
    });
    assert!(run(&mut app, Command::TerminalCopy));
    assert!(run(&mut app, Command::TerminalPaste));
    assert_eq!(*seen.borrow(), ["copy", "paste"]);
}
