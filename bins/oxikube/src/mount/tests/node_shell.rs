//! Node shells on the real init path (E09-S09): `s` on a node's row reaches the app's bus and
//! guard, asks to confirm with the node and the image named, dry-runs the pod through the guard,
//! audits it and opens a terminal tab in the cluster tab's bottom dock; a read-only cluster
//! refuses it for every initiator; the command is on the bus with an unsafe tool stub.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_terminal::view::{
    BackendDescriptor, Launch, TerminalLauncher, TerminalServices, TerminalView,
};
use oxikube_testkit::TestPorts;
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_workspace::DockPosition;
use oxikube_workspace::modal::DialogModal;

use super::App;
use crate::app_state::AppState;

/// Starts a `FakeTerminalBackend` for each launch and keeps what it was asked.
#[derive(Default)]
struct FakeLauncher {
    launches: RefCell<Vec<BackendDescriptor>>,
}

impl TerminalLauncher for FakeLauncher {
    fn launch(
        &self,
        descriptor: &BackendDescriptor,
        _: oxikube_ports::TerminalSize,
        _: &mut gpui::App,
    ) -> Launch {
        self.launches.borrow_mut().push(descriptor.clone());
        gpui::Task::ready(Ok(Box::new(FakeTerminalBackend::silent())))
    }
}

fn nodes_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Node"),
        preferred: true,
        plural: "nodes".into(),
        singular: "node".into(),
        short_names: vec!["no".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: false,
    }
}

fn node_ref() -> ResourceRef {
    ResourceRef::cluster_scoped(
        TestPorts::cluster_id(),
        Gvk::new("", "v1", "Node"),
        &*oxikube_testkit::fixtures::node_ready().meta.name,
    )
}

impl App {
    /// A nodes table in the connected cluster's tab, the first row under the cursor.
    fn open_nodes_table(&mut self) {
        self.serve([nodes_kind()]);
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
            .insert(oxikube_testkit::fixtures::node_ready());
        self.press("enter");
        self.tick();
        self.click("sidebar-entry-nodes/nodes");
        self.tick();
        let ws = self.tab_workspace();
        let table = self
            .vcx
            .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
        self.press("down");
    }

    fn confirmation(&mut self) -> Option<(String, String)> {
        let ws = self.workspace();
        let tab = self.tab_workspace();
        let _ = ws;
        self.vcx.update(|_, cx| {
            let dialog = tab
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()?;
            let dialog = dialog.read(cx);
            Some((
                dialog.title().to_string(),
                dialog.message_text()?.to_string(),
            ))
        })
    }
}

fn started(cx: &mut TestAppContext) -> (App, Rc<FakeLauncher>) {
    let launcher = Rc::new(FakeLauncher::default());
    let installed = launcher.clone();
    let mut app = App::start_with(cx, TestPorts::seeded(), move |cx| {
        oxikube_terminal::view::install(TerminalServices::new(installed), cx);
    });
    app.open_nodes_table();
    (app, launcher)
}

fn terminals(app: &mut App) -> Vec<Option<DockPosition>> {
    let ws = app.tab_workspace();
    app.vcx.update(|_, cx| {
        let ws = ws.read(cx);
        ws.items_of_type::<TerminalView>()
            .iter()
            .map(|view| ws.item_dock(view.entity_id(), cx))
            .collect()
    })
}

#[gpui::test]
fn the_node_shell_command_is_on_the_bus_as_an_unsafe_stub_hidden_from_agents(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(bus.owner(CommandId::NODE_SHELL), Some("oxikube_terminal"));
    let tool = bus.tool(CommandId::NODE_SHELL).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.node_shell");
    assert!(tool.annotations.unsafe_ && tool.annotations.interactive);
    assert!(
        !bus.agent_tools(false)
            .any(|t| t.name.as_str() == "k8s.node_shell")
    );
    assert!(
        bus.agent_tools(true)
            .any(|t| t.name.as_str() == "k8s.node_shell")
    );
}

#[gpui::test]
fn s_on_a_nodes_row_confirms_then_opens_an_audited_shell_in_the_bottom_dock(
    cx: &mut TestAppContext,
) {
    let (mut app, launcher) = started(cx);
    app.press("s");
    app.tick();

    // The guard asks first, naming the node and the image; nothing was created yet.
    let (title, message) = app.confirmation().expect("a confirmation dialog");
    assert_eq!(title, "Open a shell on the node?");
    assert!(message.contains(&*node_ref().name), "{message}");
    assert!(message.contains("busybox:1.37"), "{message}");
    assert!(message.contains("kube-system"), "{message}");
    assert!(launcher.launches.borrow().is_empty());
    assert!(app.ports.state.audit_log().is_empty());

    app.press("enter");
    app.tick();
    app.tick();

    let launches = launcher.launches.borrow().clone();
    assert!(
        matches!(launches.as_slice(), [BackendDescriptor::NodeShell { node }] if *node == node_ref()),
        "{launches:?}"
    );
    assert_eq!(
        terminals(&mut app),
        [Some(DockPosition::Bottom)],
        "a terminal tab in the bottom dock"
    );
    // The guard validated the pod with the server, and only validated it.
    let writes = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources
        .mutating_calls();
    assert!(
        writes.len() == 1 && writes[0].is_dry_run(),
        "one dry-run create: {writes:?}"
    );

    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "node::Shell");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(
        audit[0].detail.as_deref(),
        Some("phase=create image=busybox:1.37 namespace=kube-system")
    );
}

#[gpui::test]
fn declining_the_confirmation_opens_nothing_and_is_audited(cx: &mut TestAppContext) {
    let (mut app, launcher) = started(cx);
    app.press("s");
    app.tick();
    assert!(app.confirmation().is_some());
    app.press("escape");
    app.tick();
    assert!(launcher.launches.borrow().is_empty());
    assert!(terminals(&mut app).is_empty());
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
}

#[gpui::test]
fn a_read_only_cluster_refuses_a_node_shell_whoever_asks(cx: &mut TestAppContext) {
    let (mut app, launcher) = started(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let cluster = TestPorts::cluster_id();
    state
        .services()
        .sessions
        .set_read_only(&cluster, true)
        .expect("the session is open");

    // The key says why and asks nothing.
    app.press("s");
    app.tick();
    app.expect_toast("This cluster is read-only");
    assert!(app.confirmation().is_none());

    // Whatever a stale menu, the palette or an agent sends, the guard refuses it and audits it.
    let bus = state.command_bus().expect("the bus");
    let shell = Command::NodeShell { target: node_ref() };
    for initiator in [Initiator::Agent, Initiator::Command, Initiator::Ui] {
        let outcome = futures::executor::block_on(
            bus.dispatch(shell.clone(), DispatchContext::new(initiator, "someone")),
        );
        assert!(
            matches!(outcome, Err(oxikube_app::DispatchError::ReadOnly { .. })),
            "{initiator}: {outcome:?}"
        );
    }
    app.tick();
    assert!(launcher.launches.borrow().is_empty());
    assert!(terminals(&mut app).is_empty());
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 3);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[gpui::test]
fn quitting_deletes_the_pod_of_an_open_node_shell_and_audits_its_end(cx: &mut TestAppContext) {
    let (mut app, _launcher) = started(cx);
    app.press("s");
    app.tick();
    app.press("enter");
    app.tick();
    app.tick();

    // The terminal view (faked here) exchanges the guard's permit for the real session.
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let service = state.exec_service().expect("the exec service").clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    let backend = runtime
        .block_on(service.open_node_shell(&node_ref()))
        .expect("the shell opens");
    let exec = app.ports.connector.ports_for(&TestPorts::cluster_id()).exec;
    assert!(
        !exec
            .recorded_calls()
            .contains(&oxikube_testkit::ExecPortCall::ReleaseNodeShells)
    );
    assert_eq!(app.ports.state.audit_log().len(), 1, "only the create");

    // GPUI drops nothing on quit: the backend (the tab's terminal) stays alive.
    cx.update(|cx| cx.shutdown());
    assert!(
        exec.recorded_calls()
            .contains(&oxikube_testkit::ExecPortCall::ReleaseNodeShells),
        "the quit told the adapter to delete the pods it still has open"
    );
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_eq!(
        audit[1].detail.as_deref(),
        Some("phase=delete image=busybox:1.37 namespace=kube-system")
    );
    drop(backend);
}
