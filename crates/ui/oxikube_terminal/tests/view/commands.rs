//! `terminal::New`, `Split`, `Close`, `Reconnect` and `Restart`: registered on a real `CommandBus`
//! with MCP tool stubs, turned into requests, and applied by `TerminalViews` in the shown
//! workspace (a cluster's bottom dock, a split, the focused terminal). The keymap's actions send
//! the same commands.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::executor::block_on;
use gpui::{App, Window};
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_app::{ClusterSessionManager, MutationGuard};
use oxikube_domain::Initiator;
use oxikube_domain::command::CommandId;
use oxikube_terminal::view::{
    self, BackendDescriptor, TerminalHost, TerminalPanel, TerminalRequest, TerminalView,
    TerminalViewSink, TerminalViews, TerminalViewsDeps, register_view_commands,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::DockPosition;

use super::*;

#[test]
fn the_commands_are_reads_with_tool_stubs_that_become_requests() {
    let (sink, mut requests) = TerminalViewSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_terminal", |r| register_view_commands(r, sink))
        .expect("registered");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    let bus = CommandBus::new(registry, guard);
    for id in [
        CommandId::TERMINAL_NEW,
        CommandId::TERMINAL_SPLIT,
        CommandId::TERMINAL_CLOSE,
        CommandId::TERMINAL_RECONNECT,
        CommandId::TERMINAL_RESTART,
    ] {
        let meta = bus
            .commands()
            .find(|meta| meta.id == id)
            .expect("registered");
        assert!(!meta.mutating, "{id} changes nothing in a cluster");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
    for (command, request) in [
        (
            Command::TerminalNew {
                cluster: Some(cluster()),
            },
            TerminalRequest::New {
                cluster: Some(cluster()),
            },
        ),
        (Command::TerminalSplit, TerminalRequest::Split),
        (Command::TerminalClose, TerminalRequest::Close),
        (Command::TerminalReconnect, TerminalRequest::Reconnect),
        (Command::TerminalRestart, TerminalRequest::Restart),
    ] {
        block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
            .expect("dispatched");
        assert_eq!(block_on(requests.next()), Some(request));
    }
}

/// One cluster whose tab is the harness's workspace, displayed.
struct TestHost {
    cluster: Option<ClusterId>,
    workspace: Entity<Workspace>,
}

impl TerminalHost for TestHost {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        self.cluster.clone()
    }

    fn workspace(&self, cluster: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        (self.cluster.as_ref() == Some(cluster)).then(|| self.workspace.clone())
    }

    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut App) {}

    fn namespace(&self, _: &ClusterId, _: &App) -> Option<String> {
        Some("shop".into())
    }
}

pub(super) fn views(h: &mut Harness, cluster: Option<ClusterId>) -> Entity<TerminalViews> {
    let (_sink, requests) = TerminalViewSink::channel();
    let deps = TerminalViewsDeps {
        host: Rc::new(TestHost {
            cluster,
            workspace: h.ws.clone(),
        }),
        window_workspace: h.ws.downgrade(),
        services: h.services.clone(),
    };
    let views = h
        .vcx
        .update(|window, cx| TerminalViews::start(deps, requests, window, cx));
    h.vcx.run_until_parked();
    views
}

pub(super) fn apply(h: &mut Harness, views: &Entity<TerminalViews>, request: TerminalRequest) {
    h.vcx
        .update(|window, cx| views.update(cx, |views, cx| views.apply(request, window, cx)));
    h.frame();
}

pub(super) fn terminals(h: &mut Harness) -> Vec<Entity<TerminalView>> {
    let ws = h.ws.clone();
    h.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<TerminalView>())
}

#[gpui::test]
fn new_opens_a_cluster_shell_in_the_bottom_dock_split_and_close_follow_the_focus(
    cx: &mut TestAppContext,
) {
    let mut h = harness(cx);
    let views = views(&mut h, Some(cluster()));
    let ws = h.ws.clone();

    apply(&mut h, &views, TerminalRequest::New { cluster: None });
    let shell = BackendDescriptor::local(Some(cluster())).in_namespace(Some("shop".into()));
    assert_eq!(
        h.launches(),
        std::slice::from_ref(&shell),
        "the shown cluster's shell"
    );
    let first = terminals(&mut h).pop().expect("a terminal");
    let (docked, panel, open) = h.vcx.update(|_, cx| {
        let ws = ws.read(cx);
        (
            ws.item_dock(first.entity_id(), cx),
            ws.panel::<TerminalPanel>().is_some(),
            ws.dock(DockPosition::Bottom, cx)
                .is_some_and(|d| d.is_open()),
        )
    });
    assert_eq!(docked, Some(DockPosition::Bottom));
    assert!(panel, "the dock got its terminal panel");
    assert!(open, "and is shown");

    // Split: the focused terminal's descriptor, in a new pane.
    apply(&mut h, &views, TerminalRequest::Split);
    assert_eq!(h.launches(), [shell.clone(), shell]);
    let all = terminals(&mut h);
    let second = all
        .iter()
        .find(|view| **view != first)
        .expect("a second terminal")
        .clone();
    let second_pane = h.vcx.update(|_, cx| {
        ws.read(cx)
            .pane_group(cx)
            .pane_for_item(second.entity_id())
            .cloned()
    });
    assert!(second_pane.is_some(), "in a centre pane");

    // Close: the focused one (the split just opened has the focus).
    apply(&mut h, &views, TerminalRequest::Close);
    assert_eq!(terminals(&mut h), [first], "the focused terminal closed");
    assert_eq!(h.backend(1).kill_count(), 1, "its process ended");
    assert_eq!(h.backend(0).kill_count(), 0);
}

#[gpui::test]
fn split_starts_in_the_directory_the_focused_shell_is_in_now(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = views(&mut h, Some(cluster()));
    apply(&mut h, &views, TerminalRequest::New { cluster: None });
    let shell = BackendDescriptor::local(Some(cluster())).in_namespace(Some("shop".into()));
    // The user `cd`s in the first terminal.
    h.backend(0).set_working_directory("/home/me/projects/shop");

    apply(&mut h, &views, TerminalRequest::Split);
    assert_eq!(
        h.launches(),
        [shell.clone(), shell.in_dir("/home/me/projects/shop")],
        "same shell and cluster, in the shell's directory now"
    );
}

#[gpui::test]
fn with_no_cluster_shown_new_opens_a_plain_shell_in_the_window(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = views(&mut h, None);
    apply(&mut h, &views, TerminalRequest::New { cluster: None });
    assert_eq!(h.launches(), [BackendDescriptor::local(None)]);
    let view = terminals(&mut h).pop().expect("a terminal");
    let ws = h.ws.clone();
    let in_pane = h.vcx.update(|_, cx| {
        ws.read(cx)
            .pane_group(cx)
            .pane_for_item(view.entity_id())
            .is_some()
    });
    assert!(in_pane, "a centre tab");
    // Close with nothing focused closes the active pane's terminal.
    h.vcx.update(|window, cx| window.blur(cx));
    apply(&mut h, &views, TerminalRequest::Close);
    assert!(terminals(&mut h).is_empty());
}

/// A command run instead of a shell (the log viewer's `kubectl logs -f`).
fn kubectl_tail() -> BackendDescriptor {
    BackendDescriptor::local(Some(cluster()))
        .in_namespace(Some("payments".into()))
        .with_shell(
            "/usr/local/bin/kubectl",
            vec!["logs".into(), "-f".into(), "web-0".into()],
        )
        .titled("logs web-0")
}

#[gpui::test]
fn open_runs_the_descriptor_in_its_clusters_bottom_dock(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = views(&mut h, Some(cluster()));
    let ws = h.ws.clone();

    apply(
        &mut h,
        &views,
        TerminalRequest::Open {
            descriptor: kubectl_tail(),
        },
    );
    assert_eq!(
        h.launches(),
        [kubectl_tail()],
        "exactly the descriptor: the program, its arguments and the cluster, not the shown namespace"
    );
    let view = terminals(&mut h).pop().expect("a terminal");
    assert_eq!(
        view.read_with(&h.vcx, |v, _| v.descriptor().clone()),
        kubectl_tail()
    );
    let docked = h
        .vcx
        .update(|_, cx| ws.read(cx).item_dock(view.entity_id(), cx));
    assert_eq!(docked, Some(DockPosition::Bottom));

    // Nothing of the process or its environment is in what a restored tab starts from.
    let saved = kubectl_tail().to_state().to_string();
    for secret in ["KUBECONFIG", "KUBE_CONTEXT", "token", "password"] {
        assert!(!saved.contains(secret), "{secret} in {saved}");
    }
    assert_eq!(
        BackendDescriptor::from_state(&kubectl_tail().to_state()),
        Some(kubectl_tail())
    );
}

#[gpui::test]
fn open_without_a_cluster_goes_to_the_windows_own_workspace(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = views(&mut h, Some(cluster()));
    let plain = BackendDescriptor::local(None).with_shell("/bin/echo", vec!["hi".into()]);
    apply(
        &mut h,
        &views,
        TerminalRequest::Open {
            descriptor: plain.clone(),
        },
    );
    assert_eq!(
        h.launches(),
        [plain],
        "never given the shown cluster's environment"
    );
}

#[test]
fn the_sink_opens_a_descriptor_until_the_window_is_gone() {
    let (sink, mut requests) = TerminalViewSink::channel();
    assert!(sink.open(kubectl_tail()));
    assert_eq!(
        block_on(requests.next()),
        Some(TerminalRequest::Open {
            descriptor: kubectl_tail()
        })
    );
    drop(requests);
    assert!(!sink.open(kubectl_tail()), "nobody to open it");
}

#[gpui::test]
fn the_keymap_actions_send_the_bus_commands(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.vcx.dispatch_action(view::New);
    h.vcx.dispatch_action(view::Split);
    h.vcx.dispatch_action(view::Close);
    h.vcx.run_until_parked();
    assert_eq!(
        *h.recorder.0.borrow(),
        [
            Command::TerminalNew { cluster: None },
            Command::TerminalSplit,
            Command::TerminalClose
        ]
    );
}

/// The shipped keymaps bind the terminal commands, the same on every platform but for the
/// modifier, and every bound name is a registered action.
#[gpui::test]
fn the_shipped_keymaps_bind_the_terminal_commands(cx: &mut TestAppContext) {
    use oxikube_assets::{KeymapPlatform, default_keymap};
    use oxikube_keymap::file::parse_keymap;
    use oxikube_keymap::{KeymapAction, KeymapLayer};

    let h = harness(cx);
    let mut vcx = h.vcx;
    for (platform, close, split, new, search, select_all, clear) in [
        (
            KeymapPlatform::MacOs,
            "cmd-w",
            "cmd-d",
            "cmd-t",
            "cmd-f",
            "cmd-a",
            "cmd-k",
        ),
        (
            KeymapPlatform::Linux,
            "ctrl-shift-w",
            "ctrl-shift-d",
            "ctrl-shift-t",
            "ctrl-shift-f",
            "ctrl-shift-a",
            "ctrl-shift-k",
        ),
        (
            KeymapPlatform::Windows,
            "ctrl-shift-w",
            "ctrl-shift-d",
            "ctrl-shift-t",
            "ctrl-shift-f",
            "ctrl-shift-a",
            "ctrl-shift-k",
        ),
    ] {
        let parsed =
            parse_keymap(default_keymap(platform), KeymapLayer::Default).expect("a keymap");
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let mut bound = Vec::new();
        for (_, section) in &parsed.sections {
            for (key, value) in &section.bindings {
                if let Ok(KeymapAction::Action { name, .. }) = KeymapAction::from_json(value)
                    && (name.starts_with("terminal::") || name.starts_with("terminal_panel::"))
                {
                    bound.push((key.clone(), name, section.context_expr().map(str::to_owned)));
                }
            }
        }
        let terminal = Some("Terminal".to_owned());
        for expected in [
            (
                "ctrl-`".to_owned(),
                "terminal_panel::TogglePanel".to_owned(),
                None,
            ),
            ("ctrl-~".to_owned(), "terminal::New".to_owned(), None),
            (
                close.to_owned(),
                "terminal::Close".to_owned(),
                terminal.clone(),
            ),
            (
                split.to_owned(),
                "terminal::Split".to_owned(),
                terminal.clone(),
            ),
            (new.to_owned(), "terminal::New".to_owned(), terminal.clone()),
            (
                search.to_owned(),
                "terminal::Search".to_owned(),
                terminal.clone(),
            ),
            (
                select_all.to_owned(),
                "terminal::SelectAll".to_owned(),
                terminal.clone(),
            ),
            (
                clear.to_owned(),
                "terminal::Clear".to_owned(),
                terminal.clone(),
            ),
            (
                "shift-pageup".to_owned(),
                "terminal::ScrollPageUp".to_owned(),
                terminal.clone(),
            ),
            (
                "shift-down".to_owned(),
                "terminal::ScrollLineDown".to_owned(),
                terminal.clone(),
            ),
            (
                "escape".to_owned(),
                "terminal::SearchClose".to_owned(),
                Some("TerminalSearch > Input".to_owned()),
            ),
            (
                (if platform == KeymapPlatform::MacOs {
                    "cmd-g"
                } else {
                    "f3"
                })
                .to_owned(),
                "terminal::SearchNext".to_owned(),
                Some("Terminal && searching".to_owned()),
            ),
        ] {
            assert!(
                bound.contains(&expected),
                "{platform:?}: {expected:?} in {bound:?}"
            );
        }
        vcx.update(|_, cx| {
            for (_, name, _) in &bound {
                assert!(cx.build_action(name, None).is_ok(), "{name} is registered");
            }
        });
    }
}

#[gpui::test]
fn restart_starts_a_local_shell_and_reconnect_asks_the_bus_for_a_pod_session(
    cx: &mut TestAppContext,
) {
    use oxikube_ports::ExitStatus;

    let mut h = harness(cx);
    let views = views(&mut h, Some(cluster()));
    apply(&mut h, &views, TerminalRequest::New { cluster: None });
    let shell = BackendDescriptor::local(Some(cluster())).in_namespace(Some("shop".into()));

    // Running: nothing to restart.
    apply(&mut h, &views, TerminalRequest::Restart);
    assert_eq!(h.launches().len(), 1);

    // The shell exits; Reconnect is not for a local shell, Restart is.
    h.backend(0).exit(ExitStatus::with_code(1));
    h.frame();
    apply(&mut h, &views, TerminalRequest::Reconnect);
    assert_eq!(h.launches().len(), 1, "a local shell does not reconnect");
    apply(&mut h, &views, TerminalRequest::Restart);
    assert_eq!(h.launches(), [shell.clone(), shell], "the same shell again");

    // A pod terminal whose connection dropped: Reconnect asks for it again, Restart does not.
    let pod = super::lifecycle::pod_shell();
    let view = h.open(pod);
    h.frame();
    let second = h.launches().len() - 1;
    h.backend(second)
        .error(oxikube_domain::OxiError::network("reset"));
    h.backend(second).exit(ExitStatus::default());
    h.frame();
    apply(&mut h, &views, TerminalRequest::Restart);
    assert_eq!(
        h.launches().len(),
        second + 1,
        "a pod session does not restart"
    );
    apply(&mut h, &views, TerminalRequest::Reconnect);
    // A pod session starts only through its command, which the guard checks and audits: the
    // terminal sent it and launched nothing itself.
    assert_eq!(h.launches().len(), second + 1);
    assert_eq!(
        *h.recorder.0.borrow(),
        [super::lifecycle::pod_shell_command()]
    );
    let disconnected = h
        .vcx
        .update(|_, cx| matches!(view.read(cx).lifecycle(), view::Lifecycle::Disconnected(_)));
    assert!(disconnected, "the old tab keeps its screen");
}

/// Every command of the terminal's input family (copy, paste, select all, clear, scrolling, search)
/// is a read with an MCP tool stub, and each one queues the request the window runs.
#[test]
fn the_input_commands_are_reads_with_tool_stubs_that_become_requests() {
    use futures::StreamExt as _;
    use oxikube_terminal::input::{
        TerminalInputCommand, TerminalInputSink, register_input_commands,
    };

    let (sink, mut requests) = TerminalInputSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_terminal", |r| register_input_commands(r, sink))
        .expect("registered");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    let bus = CommandBus::new(registry, guard);
    let commands = [
        (Command::TerminalCopy, TerminalInputCommand::Copy),
        (Command::TerminalPaste, TerminalInputCommand::Paste),
        (Command::TerminalSelectAll, TerminalInputCommand::SelectAll),
        (Command::TerminalClear, TerminalInputCommand::Clear),
        (
            Command::TerminalScrollPageUp,
            TerminalInputCommand::ScrollPageUp,
        ),
        (
            Command::TerminalScrollPageDown,
            TerminalInputCommand::ScrollPageDown,
        ),
        (
            Command::TerminalScrollLineUp,
            TerminalInputCommand::ScrollLineUp,
        ),
        (
            Command::TerminalScrollLineDown,
            TerminalInputCommand::ScrollLineDown,
        ),
        (Command::TerminalSearch, TerminalInputCommand::Search),
        (
            Command::TerminalSearchNext,
            TerminalInputCommand::SearchNext,
        ),
        (
            Command::TerminalSearchPrevious,
            TerminalInputCommand::SearchPrevious,
        ),
        (
            Command::TerminalSearchClose,
            TerminalInputCommand::SearchClose,
        ),
    ];
    assert_eq!(commands.len(), TerminalInputCommand::ALL.len());
    for (command, request) in commands {
        let id = command.id();
        assert_eq!(request.id(), id, "the request names its command");
        assert_eq!(TerminalInputCommand::of(&command), Some(request));
        let meta = bus
            .commands()
            .find(|meta| meta.id == id)
            .expect("registered");
        assert!(!meta.mutating, "{id} changes nothing in a cluster");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
        block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
            .unwrap_or_else(|error| panic!("{id}: {error:?}"));
        assert_eq!(block_on(requests.next()), Some(request));
    }
    assert_eq!(TerminalInputCommand::of(&Command::TerminalSplit), None);
}
