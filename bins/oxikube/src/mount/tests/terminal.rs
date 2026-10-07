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
pub(super) struct FakeLauncher {
    pub(super) launches: std::cell::RefCell<Vec<oxikube_terminal::view::BackendDescriptor>>,
    backends: std::cell::RefCell<Vec<oxikube_testkit::fakes::FakeTerminalBackend>>,
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
        self.backends.borrow_mut().push(backend.clone());
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
        CommandId::TERMINAL_RECONNECT,
        CommandId::TERMINAL_RESTART,
        // E09-S11: the terminal's own actions are commands too.
        CommandId::TERMINAL_COPY,
        CommandId::TERMINAL_PASTE,
        CommandId::TERMINAL_SELECT_ALL,
        CommandId::TERMINAL_CLEAR,
        CommandId::TERMINAL_SCROLL_PAGE_UP,
        CommandId::TERMINAL_SCROLL_PAGE_DOWN,
        CommandId::TERMINAL_SCROLL_LINE_UP,
        CommandId::TERMINAL_SCROLL_LINE_DOWN,
        CommandId::TERMINAL_SEARCH,
        CommandId::TERMINAL_SEARCH_NEXT,
        CommandId::TERMINAL_SEARCH_PREVIOUS,
        CommandId::TERMINAL_SEARCH_CLOSE,
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

/// E09-S12: a shell that exited shows its banner, and `terminal::Restart` (what the banner's button
/// and the palette send) starts a fresh shell of the same descriptor in the same tab.
#[gpui::test]
fn terminal_restart_on_the_bus_starts_the_exited_shell_again(cx: &mut TestAppContext) {
    use oxikube_ports::ExitStatus;
    use oxikube_terminal::view::{BackendDescriptor, Lifecycle, TerminalView};

    let (mut app, launcher) = connected_with_fake_launcher(cx);
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    let workspace = app.vcx.update(|_, cx| tab.read(cx).workspace().clone());
    assert!(run(&mut app, Command::TerminalNew { cluster: None }));
    let view = app
        .vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<TerminalView>().pop())
        .expect("a terminal");

    // Running: the commands change nothing.
    assert!(run(&mut app, Command::TerminalRestart));
    assert!(run(&mut app, Command::TerminalReconnect));
    assert_eq!(launcher.launches.borrow().len(), 1);

    launcher.backends.borrow()[0].exit(ExitStatus::with_code(1));
    app.vcx.run_until_parked();
    app.vcx
        .executor()
        .advance_clock(oxikube_runtime::FRAME_INTERVAL);
    app.vcx.run_until_parked();
    let banner = app
        .vcx
        .update(|_, cx| view.read(cx).banner())
        .expect("the exit banner");
    assert_eq!(banner.headline, "Shell exited with code 1");

    assert!(
        run(&mut app, Command::TerminalReconnect),
        "a no-op for a shell"
    );
    assert_eq!(launcher.launches.borrow().len(), 1);
    assert!(run(&mut app, Command::TerminalRestart));
    let cluster = TestPorts::cluster_id();
    assert_eq!(
        *launcher.launches.borrow(),
        [
            BackendDescriptor::local(Some(cluster.clone())),
            BackendDescriptor::local(Some(cluster))
        ],
        "the same shell again"
    );
    let running = app
        .vcx
        .update(|_, cx| matches!(view.read(cx).lifecycle(), Lifecycle::Running));
    assert!(running);
}

/// The platform's chord for `terminal::Search` in the shipped keymap.
fn search_chord() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-shift-f"
    }
}

#[gpui::test]
fn terminal_search_and_the_settings_are_reachable_in_the_running_app(cx: &mut TestAppContext) {
    // E09-S11: a terminal opened in the cluster tab's bottom dock answers the shipped keymap's
    // search chord with its find bar, the bar's commands go through the bus, and the terminal
    // settings are in the app's settings store with their defaults.
    use oxikube_terminal::TerminalSettings;
    use oxikube_terminal::view::{BackendDescriptor, TerminalView};

    let (mut app, launcher) = connected_with_fake_launcher(cx);
    assert!(run(&mut app, Command::TerminalNew { cluster: None }));
    assert_eq!(launcher.launches.borrow().len(), 1);
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    let workspace = app.vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let view = app
        .vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<TerminalView>().pop())
        .expect("the terminal");
    assert!(!app.vcx.update(|_, cx| view.read(cx).search_open()));

    // The new terminal has the keyboard: the chord opens the bar...
    app.press(search_chord());
    assert!(
        app.vcx.update(|_, cx| view.read(cx).search_open()),
        "{} opens the find bar",
        search_chord()
    );
    // ...and escape in its field closes it, through terminal::SearchClose on the bus.
    app.press("escape");
    assert!(!app.vcx.update(|_, cx| view.read(cx).search_open()));
    // The same through the bus (palette, agents).
    assert!(run(&mut app, Command::TerminalSearch));
    assert!(app.vcx.update(|_, cx| view.read(cx).search_open()));
    assert!(run(&mut app, Command::TerminalSearchClose));
    assert!(!app.vcx.update(|_, cx| view.read(cx).search_open()));

    let settings = app.vcx.update(|_, cx| TerminalSettings::current(cx));
    assert_eq!(
        settings,
        TerminalSettings::default(),
        "default.json is the built-in default"
    );
    assert_eq!(
        *launcher.launches.borrow(),
        [BackendDescriptor::local(Some(TestPorts::cluster_id()))]
    );
}
