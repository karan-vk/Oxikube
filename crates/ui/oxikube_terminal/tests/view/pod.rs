//! Pod terminals (E09-S08): `pod::Shell`, `pod::Attach` and `pod::Exec` on a real `CommandBus`
//! (guarded, audited, with unsafe tool stubs), applied as terminal tabs in the cluster's bottom
//! dock, and never restored, cloned or retried around the bus.

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, DispatchError};
use oxikube_app::session::SessionOptions;
use oxikube_app::{ClusterSessionManager, MutationGuard};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{ErrorKind, Initiator, OxiError};
use oxikube_ports::{ClusterContext, ClusterPrefs, SourceId};
use oxikube_terminal::view::{
    BackendDescriptor, TerminalRequest, TerminalViewSink, TerminalViews, register_pod_commands,
    register_view_commands,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_ui::IconName;
use oxikube_workspace::DockPosition;

use super::*;

fn pod() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0")
}

fn shell_in(container: Option<&str>) -> BackendDescriptor {
    BackendDescriptor::Exec {
        pod: pod(),
        container: container.map(str::to_owned),
        command: Vec::new(),
    }
}

/// A bus with the terminal commands over a connected cluster, and the queue they feed.
struct Bus {
    bus: CommandBus,
    requests: futures::channel::mpsc::UnboundedReceiver<TerminalRequest>,
    audit: Arc<FakeStatePort>,
    sessions: ClusterSessionManager,
}

fn bus(read_only: bool, exec_in_read_only: bool) -> Bus {
    let (sink, requests) = TerminalViewSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_terminal", |r| {
            register_view_commands(r, sink.clone())?;
            register_pod_commands(r, sink)
        })
        .expect("registered");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let context = ClusterContext::new(
        cluster(),
        ContextName::new("kind-dev"),
        SourceId("kubeconfig".into()),
    );
    sessions.open(&context, SessionOptions::default());
    let prefs = oxikube_ports::ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
        cluster(),
        ClusterPrefs {
            read_only,
            exec_in_read_only,
            ..ClusterPrefs::default()
        },
    );
    sessions.set_prefs_table(prefs);
    block_on(sessions.connect(&cluster())).expect("connected");
    let audit = Arc::new(FakeStatePort::new());
    let guard = MutationGuard::new(sessions.clone(), audit.clone(), clock);
    Bus {
        bus: CommandBus::new(registry, guard),
        requests,
        audit,
        sessions,
    }
}

impl Bus {
    fn dispatch(&mut self, command: Command) -> Result<(), DispatchError> {
        block_on(
            self.bus
                .dispatch(command, DispatchContext::new(Initiator::Ui, "me")),
        )
        .map(|_| ())
    }

    fn next(&mut self) -> Option<TerminalRequest> {
        self.requests.try_recv().ok()
    }
}

#[test]
fn the_pod_commands_become_requests_and_are_audited() {
    let mut b = bus(false, false);
    b.dispatch(Command::PodShell {
        target: pod(),
        container: Some("app".into()),
    })
    .unwrap();
    assert_eq!(b.next(), Some(TerminalRequest::Pod(shell_in(Some("app")))));
    b.dispatch(Command::PodShell {
        target: pod(),
        container: Some("  ".into()),
    })
    .unwrap();
    assert_eq!(
        b.next(),
        Some(TerminalRequest::Pod(shell_in(None))),
        "a blank name is none"
    );
    b.dispatch(Command::PodAttach {
        target: pod(),
        container: None,
    })
    .unwrap();
    assert_eq!(
        b.next(),
        Some(TerminalRequest::Pod(BackendDescriptor::Attach {
            pod: pod(),
            container: None
        }))
    );
    b.dispatch(Command::PodExec {
        target: pod(),
        container: Some("db".into()),
        command: vec!["psql".into(), "-U".into(), "app".into()],
    })
    .unwrap();
    assert_eq!(
        b.next(),
        Some(TerminalRequest::Pod(BackendDescriptor::Exec {
            pod: pod(),
            container: Some("db".into()),
            command: vec!["psql".into(), "-U".into(), "app".into()],
        }))
    );
    // Every open is in the audit log, with the program only.
    let log = b.audit.audit_log();
    assert_eq!(log.len(), 4);
    assert_eq!(
        log[3].detail.as_deref(),
        Some("session=exec container=db program=psql")
    );
    assert!(
        log.iter()
            .all(|r| r.initiator == Initiator::Ui && r.target == pod())
    );
}

#[test]
fn a_target_that_is_not_a_pod_is_refused_without_opening_anything() {
    let mut b = bus(false, false);
    let deployment = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    );
    let err = b
        .dispatch(Command::PodShell {
            target: deployment,
            container: None,
        })
        .unwrap_err();
    assert!(
        matches!(&err, DispatchError::Handler(e) if e.kind() == ErrorKind::Validation),
        "{err:?}"
    );
    let err = b
        .dispatch(Command::PodExec {
            target: pod(),
            container: None,
            command: vec![" ".into()],
        })
        .unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)));
    assert_eq!(b.next(), None);
}

#[test]
fn read_only_blocks_the_terminal_unless_the_cluster_allows_it() {
    let mut blocked = bus(true, false);
    let err = blocked
        .dispatch(Command::PodShell {
            target: pod(),
            container: None,
        })
        .unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }), "{err:?}");
    assert_eq!(blocked.next(), None, "no terminal was queued");
    assert_eq!(blocked.audit.audit_log().len(), 1, "the refusal is audited");

    let mut allowed = bus(true, true);
    allowed
        .dispatch(Command::PodShell {
            target: pod(),
            container: None,
        })
        .unwrap();
    assert_eq!(allowed.next(), Some(TerminalRequest::Pod(shell_in(None))));
    // Read-only mode itself did not change: only the shell is let through.
    assert!(allowed.sessions.is_read_only(&cluster()));
}

#[test]
fn the_tool_stubs_are_unsafe_interactive_and_hidden_from_agents() {
    let b = bus(false, false);
    for id in [
        CommandId::POD_SHELL,
        CommandId::POD_ATTACH,
        CommandId::POD_EXEC,
    ] {
        let tool = b.bus.tool(id).expect("a stub");
        assert!(tool.annotations.unsafe_ && tool.annotations.interactive);
        assert!(!tool.agent_exposed_by_default(), "{}", tool.name);
    }
    assert_eq!(
        b.bus.tool(CommandId::POD_EXEC).unwrap().name.as_str(),
        "k8s.pod_exec"
    );
    assert_eq!(
        b.bus
            .agent_tools(false)
            .filter(|t| t.annotations.interactive)
            .count(),
        0
    );
}

// --- the tab -------------------------------------------------------------------------------

fn pod_views(h: &mut Harness) -> Entity<TerminalViews> {
    super::commands::views(h, Some(cluster()))
}

#[gpui::test]
fn a_pod_request_opens_a_terminal_tab_in_the_bottom_dock(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = pod_views(&mut h);
    super::commands::apply(&mut h, &views, TerminalRequest::Pod(shell_in(Some("app"))));
    assert_eq!(h.launches(), [shell_in(Some("app"))]);

    let terminal = super::commands::terminals(&mut h)
        .pop()
        .expect("a terminal");
    let ws = h.ws.clone();
    let docked = h
        .vcx
        .update(|_, cx| ws.read(cx).item_dock(terminal.entity_id(), cx));
    assert_eq!(docked, Some(DockPosition::Bottom));
    let content = h.vcx.update(|_, cx| {
        use oxikube_workspace::Item as _;
        terminal.read(cx).tab_content(cx)
    });
    assert_eq!(content.title.as_ref(), "web-0/app");
    assert_eq!(content.icon, Some(IconName::Container));
    assert!(content.dirty, "a session runs");
}

#[gpui::test]
fn the_tab_shows_connecting_in_its_first_frame_while_the_session_opens(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        FakeLauncher {
            hang: true,
            ..FakeLauncher::default()
        },
    );
    let view = h.open(shell_in(Some("app")));
    // The very first draw already says what is happening: connecting never leaves a blank tab.
    assert!(h.drawn("terminal-starting"));
    assert!(view.read_with(&h.vcx, |v, _| v.terminal().is_none()
        && v.failure().is_none()));
    let content = h.vcx.update(|_, cx| {
        use oxikube_workspace::Item as _;
        view.read(cx).tab_content(cx)
    });
    assert_eq!(content.title.as_ref(), "web-0/app");
    assert!(!content.dirty, "nothing runs until it connects");
}

#[gpui::test]
fn a_failed_pod_start_says_why_and_offers_retry_through_the_command(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        FakeLauncher {
            fail_next: std::cell::RefCell::new(Some(OxiError::unsupported(
                "No shell (bash, sh) was found in container app. Open a debug container instead.",
            ))),
            ..FakeLauncher::default()
        },
    );
    let view = h.open(shell_in(Some("app")));
    assert!(h.drawn("terminal-failed"), "the failure shows");
    assert!(h.drawn("terminal-retry"), "with a Retry");
    let failure = view
        .read_with(&h.vcx, |v, _| v.failure().cloned())
        .expect("failed");
    assert!(failure.contains("debug container"), "{failure}");

    // Retry sends the command again (the guard applies the policy and audits it) and closes the
    // failed tab; it does not start a process by itself.
    h.vcx.update(|_, cx| view.update(cx, |v, cx| v.retry(cx)));
    h.frame();
    assert_eq!(
        *h.recorder.0.borrow(),
        [Command::PodShell {
            target: pod(),
            container: Some("app".into())
        }]
    );
    assert_eq!(h.launches().len(), 1, "no second launch around the bus");
    assert!(
        super::commands::terminals(&mut h).is_empty(),
        "the failed tab closed"
    );
}

#[gpui::test]
fn a_failed_local_shell_retries_in_place(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        FakeLauncher {
            fail_next: std::cell::RefCell::new(Some(OxiError::internal("no pty"))),
            ..FakeLauncher::default()
        },
    );
    let view = h.open(BackendDescriptor::local(None));
    assert!(h.drawn("terminal-failed"));
    h.vcx.update(|_, cx| view.update(cx, |v, cx| v.retry(cx)));
    h.frame();
    assert_eq!(h.launches().len(), 2, "started again");
    assert!(
        view.read_with(&h.vcx, |v, _| v.terminal().is_some()),
        "and running"
    );
    assert!(
        h.recorder.0.borrow().is_empty(),
        "a local shell needs no command"
    );
}

#[gpui::test]
fn splitting_a_pod_terminal_asks_for_a_new_session_through_the_command(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = pod_views(&mut h);
    super::commands::apply(&mut h, &views, TerminalRequest::Pod(shell_in(Some("app"))));
    super::commands::apply(&mut h, &views, TerminalRequest::Split);
    assert_eq!(
        h.launches().len(),
        1,
        "the split did not start a session itself"
    );
    assert_eq!(
        *h.recorder.0.borrow(),
        [Command::PodShell {
            target: pod(),
            container: Some("app".into())
        }],
        "it sent the pod command, which the guard checks and audits"
    );
    let first = super::commands::terminals(&mut h)
        .pop()
        .expect("a terminal");
    let clone = h.vcx.update(|window, cx| {
        use oxikube_workspace::Item as _;
        first.update(cx, |view, cx| view.clone_on_split(window, cx))
    });
    assert!(
        clone.is_none(),
        "a tab dragged to split is not cloned either"
    );
}

#[gpui::test]
fn a_pod_terminal_is_never_saved_with_the_layout(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let pod_view = h.open(shell_in(Some("app")));
    let saved = h.vcx.update(|_, cx| {
        use oxikube_workspace::Item as _;
        pod_view.read(cx).serialize(cx)
    });
    assert_eq!(
        saved, None,
        "restoring would open a session nobody asked for"
    );
    let ws = h.ws.clone();
    let layout = h.vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));
    let json = serde_json::to_string(&layout.to_json()).expect("json");
    assert!(!json.contains("web-0") && !json.contains("shop"), "{json}");
}
