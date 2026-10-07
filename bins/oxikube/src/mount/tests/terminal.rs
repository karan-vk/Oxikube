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
fn terminal_copy_and_paste_are_bus_commands(cx: &mut TestAppContext) {
    // E09-S06: both reach the window's focused terminal; with none focused they do nothing, but
    // the commands exist, pass the guard and are not an error (the palette and agents use them).
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(run(&mut app, Command::TerminalCopy));
    assert!(run(&mut app, Command::TerminalPaste));
}
