//! Applying a manifest in the running app (E10-S08): `resource::Apply` is the app's guarded
//! handler on the bus; in a cluster tab the apply key dry-runs the editor's buffer through the
//! bus, shows the diff preview in the window's modal layer, and Apply writes it with a
//! server-side apply to the connected cluster (the seeded fakes), audited.

use gpui::TestAppContext;
use oxikube_domain::Risk;
use oxikube_domain::command::CommandId;
use oxikube_editor::view::{ApplyPreviewDialog, ManifestEditor, VALIDATION_DEBOUNCE};
use oxikube_ports::PatchKind;
use oxikube_testkit::TestPorts;
use oxikube_testkit::fakes::ResourceCall;

use super::App;
use crate::app_state::AppState;

fn apply_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    }
}

fn new_manifest_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-e"
    } else {
        "ctrl-shift-e"
    }
}

#[gpui::test]
fn resource_apply_is_the_apps_guarded_handler_with_a_tool(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(
        bus.owner(CommandId::RESOURCE_APPLY),
        Some("oxikube_app::apply")
    );
    let tool = bus.tool(CommandId::RESOURCE_APPLY).expect("a tool stub");
    assert_eq!(tool.name.as_str(), "k8s.resource_apply");
    assert!(tool.is_mutating());
    assert_eq!(tool.risk, Some(Risk::Low));
}

#[gpui::test]
fn the_apply_key_previews_the_dry_run_and_applies_to_the_cluster(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let resources = ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    let mut app = App::start(cx, ports);
    app.press("enter");
    let workspace = app.tab_workspace();
    app.press(new_manifest_key());
    let editor = app
        .vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<ManifestEditor>())
        .pop()
        .expect("the editor opened in the cluster tab");
    assert!(app.vcx.update(|_, cx| editor.read(cx).can_apply()));
    app.vcx.simulate_input(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: applied\n  namespace: default\ndata:\n  a: b\n",
    );
    app.vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    app.vcx.run_until_parked();

    app.press(apply_key());
    assert!(
        app.read(|ws, cx| ws
            .modal_layer()
            .read(cx)
            .active_modal::<ApplyPreviewDialog>())
            .is_some(),
        "the preview opened in the window's modal layer"
    );
    assert!(app.drawn("apply-preview-diff"));
    let writes = resources.mutating_calls();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].is_dry_run());

    app.click("apply-preview-confirm");
    let writes = resources.mutating_calls();
    let ResourceCall::Patch { patch, options, .. } = writes.last().expect("the apply") else {
        panic!("a server-side apply: {writes:?}")
    };
    assert!(!options.dry_run);
    assert!(
        matches!(&patch.kind, PatchKind::Apply { manager, force: false } if manager == "oxikube")
    );
    assert!(
        resources
            .objects()
            .iter()
            .any(|object| object.name() == "applied")
    );
    assert!(
        app.read(|ws, cx| ws
            .modal_layer()
            .read(cx)
            .active_modal::<ApplyPreviewDialog>())
            .is_none()
    );
}
