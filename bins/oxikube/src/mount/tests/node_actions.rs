//! Nodes on the real init path (E12-S09): the sidebar's Nodes entry opens the table with the
//! node columns, `c` and `u` reach the app's bus and guard, ask for one click, send a dry run and
//! then the `spec.unschedulable` patch, and audit it; a read-only cluster refuses both for every
//! initiator; the detail drawer carries the node panel and its button. The commands are on the
//! bus with mutating tool stubs.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::NodeSummary;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{PatchKind, ResourceReader as _};
use oxikube_resources_ui::detail::DetailView;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{ResourceCall, TestPorts};
use oxikube_workspace::modal::DialogModal;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

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

pub(super) fn node_name() -> String {
    oxikube_testkit::fixtures::node_ready()
        .meta
        .name
        .to_string()
}

pub(super) fn node_ref() -> ResourceRef {
    ResourceRef::cluster_scoped(
        TestPorts::cluster_id(),
        Gvk::new("", "v1", "Node"),
        node_name(),
    )
}

impl App {
    /// The nodes table of the connected cluster's tab, the first row under the cursor.
    pub(super) fn open_nodes(&mut self) -> gpui::Entity<ResourceTable> {
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
        table
    }

    fn node_dialog(&mut self) -> Option<(String, String)> {
        let tab = self.tab_workspace();
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

    pub(super) fn cluster_ports(&self) -> oxikube_testkit::FakeClusterPorts {
        self.ports.connector.ports_for(&TestPorts::cluster_id())
    }

    fn stored_node(&mut self) -> NodeSummary {
        let resources = self.cluster_ports().resources;
        let node = futures::executor::block_on(resources.get(
            &Gvk::new("", "v1", "Node"),
            None,
            &node_name(),
        ))
        .expect("the node");
        NodeSummary::from_resource(&node).expect("a node")
    }
}

/// `(dry run, body)` of the node patches the cluster received.
fn patches(app: &App) -> Vec<(bool, serde_json::Value)> {
    app.cluster_ports()
        .resources
        .mutating_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => {
                assert_eq!(patch.kind, PatchKind::Merge);
                Some((options.dry_run, patch.body))
            }
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_nodes_entry_opens_the_story_s_columns(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let table = app.open_nodes();
    let columns = app.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        })
    });
    // The warning badge column (E14-S04) comes right after the name.
    assert_eq!(columns[1], "warnings");
    let columns: Vec<_> = columns.into_iter().filter(|id| id != "warnings").collect();
    assert_eq!(
        columns,
        [
            "name",
            "status",
            "roles",
            "version",
            "os-image",
            "kernel",
            "internal-ip",
            "pods",
            "cpu",
            "cpu-percent",
            "memory",
            "memory-percent",
            "taints",
            "schedulable",
            "age"
        ]
    );
}

#[gpui::test]
fn c_asks_once_then_dry_runs_and_cordons_and_u_uncordons(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let _table = app.open_nodes();
    assert!(app.stored_node().schedulable);

    app.press("c");
    app.tick();
    let (title, message) = app.node_dialog().expect("a confirmation dialog");
    assert_eq!(title, "Are you sure?");
    assert!(
        message.starts_with("Cordon Node: Node ") && message.contains(&node_name()),
        "{message}"
    );
    assert!(patches(&app).is_empty(), "nothing is sent before the click");
    app.press("enter");
    app.tick();
    app.tick();
    let cordon = json!({"spec": {"unschedulable": true}});
    assert_eq!(patches(&app), [(true, cordon.clone()), (false, cordon)]);
    assert!(!app.stored_node().schedulable);

    app.press("u");
    app.tick();
    assert!(app.node_dialog().is_some(), "uncordon asks too");
    app.press("enter");
    app.tick();
    app.tick();
    let uncordon = json!({"spec": {"unschedulable": null}});
    assert_eq!(patches(&app).len(), 4);
    assert_eq!(patches(&app)[3], (false, uncordon));
    assert!(app.stored_node().schedulable);

    let audit = app.ports.state.audit_log();
    let seen: Vec<_> = audit
        .iter()
        .map(|r| (&*r.cmd, r.outcome, r.initiator))
        .collect();
    assert_eq!(
        seen,
        [
            ("node::Cordon", AuditOutcome::Succeeded, Initiator::Ui),
            ("node::Uncordon", AuditOutcome::Succeeded, Initiator::Ui),
        ]
    );
    assert!(audit.iter().all(|r| r.target == node_ref()));
}

#[gpui::test]
fn declining_the_confirmation_sends_nothing(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let _table = app.open_nodes();
    app.press("c");
    app.tick();
    assert!(app.node_dialog().is_some());
    app.press("escape");
    app.tick();
    assert!(patches(&app).is_empty());
    assert!(app.stored_node().schedulable);
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
}

#[gpui::test]
fn a_read_only_cluster_refuses_both_whoever_asks(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let _table = app.open_nodes();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("the session is open");

    app.press("c");
    app.tick();
    app.expect_toast("This cluster is read-only");
    assert!(app.node_dialog().is_none());

    let bus = state.command_bus().expect("the bus");
    for command in [
        Command::NodeCordon { target: node_ref() },
        Command::NodeUncordon { target: node_ref() },
    ] {
        let denied = futures::executor::block_on(
            bus.dispatch(command, DispatchContext::new(Initiator::Agent, "agent")),
        )
        .expect_err("refused");
        assert_eq!(
            denied.read_only_cluster(),
            Some(&TestPorts::cluster_id()),
            "{denied:?}"
        );
    }
    assert!(patches(&app).is_empty(), "no request left");
    let denials: Vec<_> = app
        .ports
        .state
        .audit_log()
        .iter()
        .map(|r| (r.outcome, r.initiator))
        .collect();
    assert_eq!(denials, [(AuditOutcome::Denied, Initiator::Agent); 2]);
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_mutating_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, name) in [
        (CommandId::NODE_CORDON, "k8s.node_cordon"),
        (CommandId::NODE_UNCORDON, "k8s.node_uncordon"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::nodes"));
        let tool = bus.tool(id).expect("a stub");
        assert_eq!(tool.name.as_str(), name);
        assert!(tool.is_mutating());
        assert!(bus.agent_tools(false).any(|t| t.name.as_str() == name));
    }
}

#[gpui::test]
fn the_detail_drawer_carries_the_node_panel_and_its_button(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let _table = app.open_nodes();
    app.press("enter");
    app.tick();
    assert!(app.drawn("detail-view"), "the drawer shows the node");
    app.tick();
    assert!(
        app.drawn("node-panel"),
        "the node panel is above the Overview"
    );
    assert!(app.drawn("node-panel-cordon"));
    let ws = app.tab_workspace();
    let is_node = app.vcx.update(|_, cx| {
        ws.read(cx)
            .panel::<oxikube_resources_ui::detail::DetailDrawer>()
            .and_then(|d| d.read(cx).view().cloned())
            .map(|view: gpui::Entity<DetailView>| view.read(cx).target().gvk.is_node())
    });
    assert_eq!(is_node, Some(true));

    app.click("node-panel-cordon");
    app.tick();
    let (_, message) = app.node_dialog().expect("the button asks first");
    assert!(message.starts_with("Cordon Node:"), "{message}");
}
