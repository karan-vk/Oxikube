//! Node shells (E09-S09) in the terminal: `node::Shell` on a real bus (read-only blocked, a
//! confirmation naming the node and the image, a server dry run, an audit record) queues a
//! terminal tab in the cluster's bottom dock, and that tab is never restored, cloned, split or
//! reconnected around the bus.

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, DispatchError};
use oxikube_app::exec::register_command;
use oxikube_app::session::SessionOptions;
use oxikube_app::{ClusterSessionManager, Confirmation, ExecService, MutationGuard, Outcome};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{Initiator, OxiError};
use oxikube_ports::{ClusterContext, ClusterPrefs, ClusterPrefsTable, SourceId};
use oxikube_terminal::view::{
    BackendDescriptor, TerminalRequest, TerminalViewSink, register_view_commands,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_ui::IconName;
use oxikube_workspace::DockPosition;

use super::*;

fn node() -> ResourceRef {
    ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "worker-1")
}

fn node_shell() -> BackendDescriptor {
    BackendDescriptor::NodeShell { node: node() }
}

struct Bus {
    bus: CommandBus,
    requests: futures::channel::mpsc::UnboundedReceiver<TerminalRequest>,
    audit: Arc<FakeStatePort>,
    connector: Arc<FakeClusterConnectorPort>,
}

fn bus(read_only: bool) -> Bus {
    let (sink, requests) = TerminalViewSink::channel();
    let clock = Arc::new(FakeClockPort::default());
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let sessions = ClusterSessionManager::new(
        connector.clone(),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let exec = Arc::new(ExecService::new(sessions.clone()));
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_terminal", |r| {
            register_view_commands(r, sink.clone())?;
            register_command(r, exec.clone(), sink.node_shell_opener())
        })
        .expect("registered");
    let context = ClusterContext::new(
        cluster(),
        ContextName::new("kind-dev"),
        SourceId("kubeconfig".into()),
    );
    sessions.open(&context, SessionOptions::default());
    sessions.set_prefs_table(
        ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            cluster(),
            ClusterPrefs {
                read_only,
                ..ClusterPrefs::default()
            },
        ),
    );
    block_on(sessions.connect(&cluster())).expect("connected");
    let audit = Arc::new(FakeStatePort::new());
    let guard = MutationGuard::new(sessions, audit.clone(), clock);
    exec.set_audit(guard.audit_handle());
    Bus {
        bus: CommandBus::new(registry, guard),
        requests,
        audit,
        connector,
    }
}

impl Bus {
    fn dispatch(&self, answer: Option<Confirmation>) -> Result<Outcome, DispatchError> {
        let mut context = DispatchContext::new(Initiator::Ui, "me");
        context.confirmation = answer;
        block_on(
            self.bus
                .dispatch(Command::NodeShell { target: node() }, context),
        )
    }

    /// Dispatches, confirms what the guard asks and runs.
    fn confirmed(&self) -> Result<Outcome, DispatchError> {
        let Outcome::NeedsConfirmation(request) = self.dispatch(None)? else {
            panic!("a node shell is always confirmed");
        };
        self.dispatch(Some(Confirmation::simple(request.token)))
    }
}

#[test]
fn a_confirmed_node_shell_queues_a_terminal_and_is_audited_with_its_image() {
    let mut b = bus(false);
    let Ok(Outcome::NeedsConfirmation(request)) = b.dispatch(None) else {
        panic!("expected a confirmation");
    };
    assert!(request.summary.contains("worker-1"), "{}", request.summary);
    assert!(
        request.summary.contains("busybox:1.37"),
        "{}",
        request.summary
    );
    assert_eq!(
        b.requests.try_recv().ok(),
        None,
        "nothing before the answer"
    );

    assert!(matches!(
        b.dispatch(Some(Confirmation::simple(request.token))),
        Ok(Outcome::Completed(_))
    ));
    assert_eq!(
        b.requests.try_recv().ok(),
        Some(TerminalRequest::Pod(node_shell()))
    );
    let log = b.audit.audit_log();
    assert_eq!(log.len(), 1);
    assert_eq!(
        log[0].detail.as_deref(),
        Some("phase=create image=busybox:1.37 namespace=kube-system")
    );
    let ports = b.connector.ports_for(&cluster());
    assert!(
        ports
            .resources
            .mutating_calls()
            .iter()
            .all(|c| c.is_dry_run()),
        "the guard only validates the pod"
    );
}

#[test]
fn a_read_only_cluster_queues_no_terminal() {
    let mut b = bus(true);
    let err = b.dispatch(None).unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }), "{err:?}");
    assert_eq!(b.requests.try_recv().ok(), None);
    assert_eq!(b.audit.audit_log().len(), 1, "the refusal is audited");
}

#[test]
fn a_refused_dry_run_queues_no_terminal() {
    let mut b = bus(false);
    b.connector
        .ports_for(&cluster())
        .resources
        .script()
        .create
        .push_err(OxiError::forbidden("violates PodSecurity"));
    let err = b.confirmed().unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)), "{err:?}");
    assert_eq!(b.requests.try_recv().ok(), None);
}

#[test]
fn the_tool_stub_is_unsafe_interactive_and_hidden_from_agents() {
    let b = bus(false);
    let tool = b.bus.tool(CommandId::NODE_SHELL).expect("a stub");
    assert!(tool.annotations.unsafe_ && tool.annotations.interactive);
    assert!(!tool.agent_exposed_by_default());
    assert_eq!(tool.name.as_str(), "k8s.node_shell");
}

// --- the tab -------------------------------------------------------------------------------

#[gpui::test]
fn a_node_shell_request_opens_a_terminal_tab_in_the_bottom_dock(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = super::commands::views(&mut h, Some(cluster()));
    super::commands::apply(&mut h, &views, TerminalRequest::Pod(node_shell()));
    assert_eq!(h.launches(), [node_shell()]);

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
    assert_eq!(content.title.as_ref(), "node/worker-1");
    assert_eq!(content.icon, Some(IconName::Container));
}

#[gpui::test]
fn the_tab_says_the_pod_is_being_created_while_it_starts(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        FakeLauncher {
            hang: true,
            ..FakeLauncher::default()
        },
    );
    let view = h.open(node_shell());
    assert!(h.drawn("terminal-starting"));
    let text = view.read_with(&h.vcx, |v, _| v.descriptor().starting_text(&v.title()));
    assert!(text.contains("worker-1"), "{text}");
    assert!(text.contains("creating the privileged shell pod"), "{text}");
    assert!(text.contains("waiting for it to start"), "{text}");
    assert!(text.contains("connecting"), "{text}");
}

#[gpui::test]
fn a_failed_node_shell_says_why_and_reconnect_asks_the_guard_again(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        FakeLauncher {
            fail_next: std::cell::RefCell::new(Some(OxiError::conflict(
                "Check that the node can pull busybox:1.37 (the `node_shell_image` setting). \
                 The shell pod was deleted.",
            ))),
            ..FakeLauncher::default()
        },
    );
    let view = h.open(node_shell());
    assert!(h.drawn("terminal-failed"));
    assert!(h.drawn("terminal-banner-Reconnect"));
    let failure = view
        .read_with(&h.vcx, |v, _| v.failure().cloned())
        .expect("failed");
    assert!(failure.contains("node_shell_image"), "{failure}");

    assert!(
        h.vcx
            .update(|_, cx| view.update(cx, |v, cx| v.reconnect(cx)))
    );
    h.frame();
    assert_eq!(
        *h.recorder.0.borrow(),
        [Command::NodeShell { target: node() }],
        "the command again: confirmed and audited like the first"
    );
    assert_eq!(h.launches().len(), 1, "no second pod around the guard");
}

#[gpui::test]
fn splitting_a_node_shell_asks_for_a_new_one_through_the_command(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let views = super::commands::views(&mut h, Some(cluster()));
    super::commands::apply(&mut h, &views, TerminalRequest::Pod(node_shell()));
    super::commands::apply(&mut h, &views, TerminalRequest::Split);
    assert_eq!(h.launches().len(), 1);
    assert_eq!(
        *h.recorder.0.borrow(),
        [Command::NodeShell { target: node() }]
    );
    let first = super::commands::terminals(&mut h)
        .pop()
        .expect("a terminal");
    let clone = h.vcx.update(|window, cx| {
        use oxikube_workspace::Item as _;
        first.update(cx, |view, cx| view.clone_on_split(window, cx))
    });
    assert!(clone.is_none(), "a dragged tab is not cloned either");
}

#[gpui::test]
fn a_node_shell_is_never_saved_with_the_layout(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let view = h.open(node_shell());
    let saved = h.vcx.update(|_, cx| {
        use oxikube_workspace::Item as _;
        view.read(cx).serialize(cx)
    });
    assert_eq!(
        saved, None,
        "restoring would create a privileged pod unasked"
    );
    let ws = h.ws.clone();
    let layout = h.vcx.update(|_, cx| ws.read(cx).serialize_layout(cx));
    let json = serde_json::to_string(&layout.to_json()).expect("json");
    assert!(!json.contains("worker-1"), "{json}");
}

#[test]
fn the_descriptor_names_the_node_and_reopens_by_its_command() {
    let descriptor = node_shell();
    assert_eq!(descriptor.cluster(), Some(&cluster()));
    assert!(!descriptor.is_local());
    assert_eq!(descriptor.default_title(None), "node/worker-1");
    assert_eq!(
        descriptor.pod_command(),
        Some(Command::NodeShell { target: node() })
    );
    let state = descriptor.to_state();
    assert_eq!(BackendDescriptor::from_state(&state), Some(descriptor));
}
