//! Restart and set image on the real init path (E12-S05): `r` on a Deployment's row asks the
//! guard's one confirmation, patches the pod template's `restartedAt` (after a server dry run),
//! audits it and offers to open the workload; `i` opens the set-image dialog, whose button sends a
//! strategic patch naming only the changed container through the app's own `CommandBus` and
//! `MutationGuard`; a read-only cluster refuses both; the drawer's header has the buttons; both
//! commands are on the bus with tool stubs.

use gpui::{Entity, TestAppContext};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_ports::PatchKind;
use oxikube_resources_ui::kinds::{SetImageDialog, SetImageStage};
use oxikube_testkit::{ResourceCall, TestPorts};
use oxikube_workspace::modal::DialogModal;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

impl App {
    fn set_image_dialog(&mut self) -> Option<Entity<SetImageDialog>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<SetImageDialog>()
        })
    }

    fn rollout_confirmation(&mut self) -> Option<(String, String)> {
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

    fn writes_of_cluster(&self) -> Vec<ResourceCall> {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
            .mutating_calls()
    }
}

#[gpui::test]
fn both_commands_are_on_the_bus_as_guarded_tool_stubs_and_are_row_actions(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let offered: Vec<_> = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .map(|spec| spec.command)
        .collect();
    for (id, tool) in [
        (CommandId::WORKLOAD_RESTART, "k8s.workload_restart"),
        (CommandId::WORKLOAD_SET_IMAGE, "k8s.workload_set_image"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::workloads"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
        assert!(offered.contains(&id), "{id} is a row action");
    }
}

#[gpui::test]
fn r_confirms_then_patches_the_template_audits_and_reports(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.press("r");
    app.tick();

    let (title, message) = app.rollout_confirmation().expect("the guard asks first");
    assert_eq!(title, "Restart the workload?");
    assert!(message.contains("demo/web"), "{message}");
    assert!(message.contains("rolling update"), "{message}");
    assert!(
        app.writes_of_cluster().is_empty(),
        "nothing is sent before the answer"
    );

    app.press("enter");
    app.tick();
    app.tick();

    // The server rehearsed the patch, then it was sent: the same merge patch both times.
    let writes = app.writes_of_cluster();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert!(writes[0].is_dry_run() && !writes[1].is_dry_run());
    let ResourceCall::Patch { patch, name, .. } = &writes[1] else {
        panic!("a patch: {writes:?}");
    };
    assert_eq!(name, "web");
    assert_eq!(patch.kind, PatchKind::Merge);
    let stamped = patch.body["spec"]["template"]["metadata"]["annotations"]
        ["kubectl.kubernetes.io/restartedAt"]
        .as_str()
        .expect("the annotation");
    assert!(
        stamped.ends_with('Z') && stamped.len() == "2026-01-01T00:00:00Z".len(),
        "{stamped}"
    );

    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "workload::Restart");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    let detail = audit[0].detail.as_deref().unwrap_or_default();
    assert!(
        detail.starts_with(&format!("restartedAt={stamped} rv=")),
        "{detail}"
    );

    let toasts = app.toasts();
    assert!(
        toasts
            .iter()
            .any(|t| t.contains("Restarting Deployment demo/web")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn declining_the_restart_changes_nothing_and_is_audited(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.press("r");
    app.tick();
    assert!(app.rollout_confirmation().is_some());
    app.press("escape");
    app.tick();
    assert!(app.writes_of_cluster().is_empty());
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
}

#[gpui::test]
fn i_opens_the_dialog_and_its_button_sends_a_strategic_patch_through_the_guard(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.press("i");
    app.tick();
    let dialog = app
        .set_image_dialog()
        .expect("i opened the set-image dialog");
    let (stage, fields) = app.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        let fields: Vec<_> = d
            .fields()
            .iter()
            .map(|f| (f.name.to_string(), d.text(&f.name, cx).unwrap()))
            .collect();
        (d.stage(), fields)
    });
    assert_eq!(stage, SetImageStage::Editing);
    assert_eq!(
        fields,
        [("app".to_owned(), "registry.example/app:1.0".to_owned())]
    );
    assert!(app.drawn("setimage-dialog"));
    assert!(app.writes_of_cluster().is_empty(), "opening only reads");

    app.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| {
            d.set_text("app", "registry.example/app:1.1", window, cx);
            d.submit(cx);
        })
    });
    app.tick();
    app.tick();
    assert!(app.set_image_dialog().is_none(), "the dialog closed");

    // The guard's dry run before it asks (E19-S07), the handler's rehearsal, the patch.
    let writes = app.writes_of_cluster();
    assert_eq!(writes.len(), 3, "{writes:?}");
    assert!(writes[0].is_dry_run() && writes[1].is_dry_run() && !writes[2].is_dry_run());
    let ResourceCall::Patch { patch, .. } = &writes[2] else {
        panic!("a patch: {writes:?}");
    };
    assert_eq!(patch.kind, PatchKind::Strategic);
    assert_eq!(
        patch.body,
        json!({"spec": {"template": {"spec": {
            "containers": [{"name": "app", "image": "registry.example/app:1.1"}]
        }}}})
    );
    let audit: Vec<_> = app
        .ports
        .state
        .audit_log()
        .into_iter()
        .filter(|r| !r.dry_run)
        .collect();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "workload::SetImage");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert!(
        audit[0].detail.as_deref().unwrap_or_default().starts_with(
            "containers=app app=registry.example/app:1.0->registry.example/app:1.1 rv="
        ),
        "{:?}",
        audit[0].detail
    );
    assert!(
        app.toasts()
            .iter()
            .any(|t| t.contains("Set the image of Deployment demo/web")),
        "{:?}",
        app.toasts()
    );
}

#[gpui::test]
fn a_read_only_cluster_refuses_both_keys_and_sends_nothing(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("a known cluster");
    app.press("r");
    app.tick();
    app.press("i");
    app.tick();
    assert!(app.rollout_confirmation().is_none(), "no confirmation");
    assert!(app.set_image_dialog().is_none(), "no dialog");
    assert!(app.writes_of_cluster().is_empty());
    assert!(
        app.toasts().iter().any(|t| t.contains("read-only")),
        "{:?}",
        app.toasts()
    );
    // The same refusal for the palette and an agent: the guard blocks every initiator.
    let bus = state.command_bus().expect("the bus");
    for initiator in Initiator::ALL {
        let command = oxikube_domain::command::Command::WorkloadRestart {
            target: oxikube_domain::ids::ResourceRef::namespaced(
                TestPorts::cluster_id(),
                oxikube_domain::ids::Gvk::new("apps", "v1", "Deployment"),
                "demo",
                "web",
            ),
        };
        let ctx = oxikube_app::command_bus::DispatchContext::new(initiator, "someone");
        let error = futures::executor::block_on(bus.dispatch(command, ctx)).unwrap_err();
        assert!(
            matches!(
                error,
                oxikube_app::command_bus::DispatchError::ReadOnly { .. }
            ),
            "{initiator}"
        );
    }
    assert!(app.writes_of_cluster().is_empty());
}

#[gpui::test]
fn the_drawer_header_has_both_buttons_and_the_restart_button_asks_to_confirm(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_deployments_table();
    app.press("enter");
    app.tick();
    app.drawn("detail-view");
    app.tick();
    assert!(app.drawn("detail-restart"), "the Restart button");
    assert!(app.drawn("detail-set-image"), "the Set image button");
    app.click("detail-restart");
    app.tick();
    let (title, _) = app.rollout_confirmation().expect("the guard asks first");
    assert_eq!(title, "Restart the workload?");
    app.press("escape");
    app.tick();
    app.click("detail-set-image");
    app.tick();
    assert!(
        app.set_image_dialog().is_some(),
        "the button opens the dialog"
    );
}
