//! Node drain on the real init path (E12-S10): `r` on the Nodes table opens the drain dialog of
//! the app's mount, which plans from the node's pods, wants the node's name typed, and drains
//! through the bus, the guard and the kube adapter's `KubeDrain` (here over the fake resource
//! port): the cordon, each eviction and the audit trail. The command is on the bus with a
//! mutating tool stub, and the node panel has its button.

use gpui::{Entity, TestAppContext};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_ports::ListPage;
use oxikube_resources_ui::kinds::node::drain::{DrainDialog, DrainLoaded, DrainStage, DrainSwitch};
use oxikube_testkit::{ResourceCall, TestPorts, pod};

use super::App;
use super::node_actions::{node_name, node_ref};
use crate::app_state::AppState;

impl App {
    fn drain_dialog(&mut self) -> Option<Entity<DrainDialog>> {
        let tab = self.tab_workspace();
        self.vcx.update(|_, cx| {
            tab.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DrainDialog>()
        })
    }

    /// Scripts what a list of the node's pods answers (the fake does not filter by field), `n`
    /// times: the dialog's read, and the drain's look before and after the cordon.
    fn script_node_pods(&self, n: usize) {
        let resources = self.cluster_ports().resources;
        for _ in 0..n {
            resources.script().list.push_ok(ListPage::complete(vec![
                pod()
                    .namespace("shop")
                    .name("web-1")
                    .uid("web-1-uid")
                    .node(node_name())
                    .running()
                    .controlled_by("ReplicaSet", "web")
                    .build(),
            ]));
        }
    }
}

#[gpui::test]
fn r_opens_the_drain_dialog_and_the_drain_runs_through_the_guard(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let _table = app.open_nodes();
    app.script_node_pods(3);
    app.cluster_ports().resources.script().evict.push_ok(());

    app.press("r");
    app.tick();
    let dialog = app.drain_dialog().expect("r opens the drain dialog");
    assert!(app.drawn("drain-dialog"));
    let (target, ready) = app.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (
            d.target().clone(),
            matches!(d.loaded_state(), DrainLoaded::Ready(_)),
        )
    });
    assert_eq!(target, node_ref());
    assert!(ready, "the node's pods were read");
    assert!(
        app.cluster_ports().resources.mutating_calls().is_empty(),
        "nothing is sent while the dialog asks"
    );

    // The name field has the focus: type the node's name, then Drain.
    app.vcx.simulate_input(&node_name());
    app.tick();
    app.vcx.update(|_, cx| {
        dialog.update(cx, |d, cx| {
            d.set_flag(DrainSwitch::IgnoreDaemonSets, true, cx);
            assert!(d.can_start(cx), "{:?}", d.start_blocker(cx));
            d.start(cx);
        });
    });
    for _ in 0..4 {
        app.tick();
    }
    let (stage, message) = app.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.stage(), d.message_text().map(str::to_owned))
    });
    assert_eq!(stage, DrainStage::Done, "{message:?}");
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.starts_with("Drained node")),
        "{message:?}"
    );

    // The cordon and the eviction reached the cluster through the guard's writer.
    let writes = app.cluster_ports().resources.mutating_calls();
    assert!(
        matches!(writes.first(), Some(ResourceCall::Patch { .. })),
        "{writes:?}"
    );
    assert!(
        writes
            .iter()
            .any(|w| matches!(w, ResourceCall::Evict { .. })),
        "{writes:?}"
    );
    let audit = app.ports.state.audit_log();
    let seen: Vec<_> = audit
        .iter()
        .map(|r| (&*r.cmd, r.target.name.to_string(), r.outcome, r.initiator))
        .collect();
    assert_eq!(
        seen,
        [
            (
                "node::Drain",
                "web-1".to_owned(),
                AuditOutcome::Succeeded,
                Initiator::Ui
            ),
            (
                "node::Drain",
                node_name(),
                AuditOutcome::Succeeded,
                Initiator::Ui
            ),
        ]
    );
}

#[gpui::test]
fn the_drain_is_on_the_bus_with_a_mutating_tool_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(bus.owner(CommandId::NODE_DRAIN), Some("oxikube_app::nodes"));
    let tool = bus.tool(CommandId::NODE_DRAIN).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.node_drain");
    assert!(tool.is_mutating() && tool.destructive_hint());
}

#[gpui::test]
fn the_node_panel_has_a_drain_button(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let _table = app.open_nodes();
    app.press("enter");
    app.tick();
    app.tick();
    assert!(app.drawn("node-panel-drain"));
    app.script_node_pods(1);
    app.click("node-panel-drain");
    app.tick();
    assert!(app.drain_dialog().is_some(), "the button opens the dialog");
}
