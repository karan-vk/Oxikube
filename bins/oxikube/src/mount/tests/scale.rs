//! Workloads on the real init path (E12-S04): the Workloads section lists Deployments,
//! StatefulSets, ReplicaSets and ReplicationControllers, `s` on a Deployment's row opens the scale
//! dialog, its button reaches the cluster through the app's own `CommandBus` and `MutationGuard`
//! (the `scale` subresource, audited with the count before and after), a read-only cluster
//! refuses, and the detail drawer opens with the workload panel and its Scale button.

use gpui::{Entity, TestAppContext};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_ports::Scale;
use oxikube_resources_ui::detail::DetailDrawer;
use oxikube_resources_ui::kinds::{ScaleDialog, ScaleStage, WorkloadPanel};
use oxikube_testkit::{ResourceCall, TestPorts};

use super::App;
use crate::app_state::AppState;

fn scale_of(spec: i32, status: i32) -> Scale {
    Scale {
        replicas: spec,
        current_replicas: status,
        selector: None,
        resource_version: None,
    }
}

impl App {
    fn scale_dialog(&mut self) -> Option<Entity<ScaleDialog>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<ScaleDialog>()
        })
    }

    fn resources(&self) -> std::sync::Arc<oxikube_testkit::FakeResourcePort> {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
    }
}

#[gpui::test]
fn scale_is_on_the_bus_as_a_guarded_tool_stub_owned_by_the_workloads_service(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(
        bus.owner(CommandId::WORKLOAD_SCALE),
        Some("oxikube_app::workloads")
    );
    let tool = bus.tool(CommandId::WORKLOAD_SCALE).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.workload_scale");
}

#[gpui::test]
fn the_sidebar_lists_the_workload_kinds_and_s_scales_through_the_guard(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.resources().script().get_scale.push_ok(scale_of(3, 3));
    app.press("s");
    app.tick();
    let dialog = app.scale_dialog().expect("s opened the scale dialog");
    let (stage, current) = app.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.stage(), d.plan().map(|p| (p.current, p.desired)))
    });
    assert_eq!(stage, ScaleStage::Editing);
    assert_eq!(current, Some((3, 3)));
    assert!(app.drawn("scale-dialog"));

    // The user types 5 and confirms.
    app.resources().script().get_scale.push_ok(scale_of(3, 3));
    app.resources().script().scale.push_ok(scale_of(5, 3));
    app.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| {
            d.set_text("5", window, cx);
            d.submit(cx);
        })
    });
    app.tick();
    app.tick();
    assert!(app.scale_dialog().is_none(), "the dialog closed");

    let writes = app.resources().mutating_calls();
    assert!(
        matches!(&writes[..], [ResourceCall::Scale { replicas: 5, name, .. }] if name == "web"),
        "{writes:?}"
    );
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "workload::Scale");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(
        audit[0].detail.as_deref(),
        Some("replicas=5 before=3 after=5")
    );
    app.expect_toast("Scaled Deployment demo/web from 3 to 5 replicas");
}

#[gpui::test]
fn a_read_only_cluster_refuses_the_key_and_sends_nothing(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("a known cluster");
    app.press("s");
    app.tick();
    assert!(app.scale_dialog().is_none());
    assert!(app.resources().mutating_calls().is_empty());
    assert!(
        app.toasts().iter().any(|t| t.contains("read-only")),
        "{:?}",
        app.toasts()
    );
}

#[gpui::test]
fn the_detail_drawer_of_a_deployment_has_the_panel_with_a_scale_button(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.press("enter");
    app.tick();
    let ws = app.tab_workspace();
    let view = app
        .vcx
        .update(|_, cx| {
            let drawer = ws.read(cx).panel::<DetailDrawer>()?;
            drawer.read(cx).view().cloned()
        })
        .expect("the detail drawer is open on the deployment");
    // Drawn once so the panel is built, then once more for its own first read.
    app.drawn("detail-view");
    app.tick();
    let panel = app
        .vcx
        .update(|_, cx| view.read(cx).kind_panel().cloned())
        .expect("a kind panel")
        .downcast::<WorkloadPanel>()
        .expect("the workload panel");
    assert!(app.drawn("workload-panel"));
    assert!(app.drawn("workload-scale"), "the Scale button");
    let stats = app.vcx.update(|_, cx| {
        panel
            .read(cx)
            .model()
            .map(|m| m.stats.iter().map(|s| s.label).collect::<Vec<_>>())
    });
    assert_eq!(
        stats,
        Some(vec!["Ready", "Up-to-date", "Available", "Replicas"])
    );

    app.resources().script().get_scale.push_ok(scale_of(3, 3));
    app.click("workload-scale");
    app.tick();
    assert!(app.scale_dialog().is_some(), "the button opens the dialog");
}
