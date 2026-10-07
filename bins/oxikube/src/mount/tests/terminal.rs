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

/// Starts a `FakeTerminalBackend` for each launch and keeps what it was asked (no process, no OS
/// thread in a GPUI test).
#[derive(Default)]
struct FakeLauncher {
    launches: std::cell::RefCell<Vec<oxikube_terminal::view::BackendDescriptor>>,
}

impl oxikube_terminal::view::TerminalLauncher for FakeLauncher {
    fn launch(
        &self,
        descriptor: &oxikube_terminal::view::BackendDescriptor,
        _: oxikube_ports::TerminalSize,
        _: &mut gpui::App,
    ) -> oxikube_terminal::view::Launch {
        self.launches.borrow_mut().push(descriptor.clone());
        let backend = oxikube_testkit::fakes::FakeTerminalBackend::silent();
        gpui::Task::ready(Ok(Box::new(backend)))
    }
}

/// The app with a fake launcher, its cluster connected (Enter in the catalog) and its tab shown.
fn connected_with_fake_launcher(cx: &mut TestAppContext) -> (App, std::rc::Rc<FakeLauncher>) {
    use oxikube_terminal::view::TerminalServices;
    let launcher = std::rc::Rc::new(FakeLauncher::default());
    let installed = launcher.clone();
    let mut app = App::start_with(cx, TestPorts::seeded(), move |cx| {
        oxikube_terminal::view::install(TerminalServices::new(installed), cx);
    });
    app.press("enter");
    (app, launcher)
}

#[gpui::test]
fn the_terminal_commands_are_on_the_bus(cx: &mut TestAppContext) {
    use oxikube_domain::command::CommandId;
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for id in [
        CommandId::TERMINAL_NEW,
        CommandId::TERMINAL_SPLIT,
        CommandId::TERMINAL_CLOSE,
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_terminal"), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}

#[gpui::test]
fn terminal_new_opens_a_cluster_shell_in_the_cluster_tab_s_bottom_dock(cx: &mut TestAppContext) {
    use oxikube_terminal::view::{BackendDescriptor, TerminalPanel, TerminalView};
    use oxikube_workspace::DockPosition;

    let (mut app, launcher) = connected_with_fake_launcher(cx);
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    let workspace = app.vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let (panel, open) = app.vcx.update(|_, cx| {
        let ws = workspace.read(cx);
        let open = ws.dock(DockPosition::Bottom, cx).map(|dock| dock.is_open());
        (ws.panel::<TerminalPanel>().is_some(), open)
    });
    assert!(panel, "every cluster tab has the terminal panel");
    assert_eq!(
        open,
        Some(false),
        "in a closed bottom dock until a terminal opens"
    );

    // The keymap, the panel's button and the palette all send this.
    assert!(run(&mut app, Command::TerminalNew { cluster: None }));

    let cluster = TestPorts::cluster_id();
    assert_eq!(
        *launcher.launches.borrow(),
        [BackendDescriptor::local(Some(cluster))],
        "a shell with the shown cluster's environment"
    );
    let (terminals, docked, open) = app.vcx.update(|_, cx| {
        let ws = workspace.read(cx);
        let terminals = ws.items_of_type::<TerminalView>();
        let docked = terminals
            .first()
            .and_then(|view| ws.item_dock(view.entity_id(), cx));
        let open = ws
            .dock(DockPosition::Bottom, cx)
            .is_some_and(|d| d.is_open());
        (terminals.len(), docked, open)
    });
    assert_eq!(terminals, 1, "in the cluster's own workspace");
    assert_eq!(docked, Some(DockPosition::Bottom));
    assert!(open, "the dock opened for it");

    // Close it again through the bus: the focused terminal goes.
    assert!(run(&mut app, Command::TerminalClose));
    let left = app
        .vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<TerminalView>().len());
    assert_eq!(left, 0);
}
