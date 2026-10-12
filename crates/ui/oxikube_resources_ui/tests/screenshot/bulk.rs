//! The bulk dialog (E12-S19), in the dark and the light theme: the label editor over three pods
//! with its preview and the selector warning, the delete confirmation of a selection with a
//! Namespace in it (counts by kind and namespace, the cluster name to type) and the results of a
//! batch where one object failed and one was already gone.
//!
//! The cluster is a fake one with five pods; the objects named in the dialogs are references, so
//! nothing here depends on the clock.

use std::sync::Arc;

use gpui::{AppContext as _, WeakEntity, px, size};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::{ClusterSessionManager, MutationGuard, SelectionFlow};
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClusterContext, DeleteOutcome, SourceId};
use oxikube_resources_ui::bulk::{BulkDialog, BulkKind, BulkSample};
use oxikube_testkit::screenshot::RgbaImage;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort, pod,
};

use super::{check, headless};

const WIDTH: f32 = 600.0;
const EDITOR_HEIGHT: f32 = 640.0;
const DELETE_HEIGHT: f32 = 400.0;
const RESULTS_HEIGHT: f32 = 400.0;

struct World {
    flow: SelectionFlow,
    cluster: ClusterId,
    connector: Arc<FakeClusterConnectorPort>,
}

fn world() -> World {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    for i in 0..5 {
        ports
            .resources
            .insert(pod().namespace("shop").name(format!("web-{i}")).build());
    }
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::selection", |r| {
            oxikube_app::selection::register_commands(r, sessions.clone())
        })
        .expect("register the bulk commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(sessions.clone(), Arc::new(FakeStatePort::new()), clock),
    );
    World {
        flow: SelectionFlow::new(bus, sessions, "alice"),
        cluster,
        connector,
    }
}

fn pod_ref(cluster: &ClusterId, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Pod"), namespace, name)
}

fn pin(cx: &mut gpui::App, light: bool) {
    oxikube_ui::init(cx);
    let tokens = if light {
        oxikube_ui::Tokens::light()
    } else {
        oxikube_ui::Tokens::dark()
    };
    oxikube_ui::set_tokens(cx, tokens);
    oxikube_runtime::init_deterministic(cx);
    cx.set_reduce_motion(true);
}

/// Opens a dialog of `kind` over `targets` in a window of `height`, lets `drive` act on it, and
/// captures the frame.
fn shot(
    w: &World,
    kind: BulkKind,
    targets: Vec<ResourceRef>,
    sample: BulkSample,
    height: f32,
    light: bool,
    drive: impl FnOnce(&mut BulkDialog, &mut gpui::Window, &mut gpui::Context<BulkDialog>),
) -> anyhow::Result<RgbaImage> {
    let command = match kind {
        BulkKind::Delete => Command::SelectionDelete {
            targets,
            propagation: Default::default(),
        },
        BulkKind::Restart => Command::SelectionRestart { targets },
        BulkKind::Label => Command::SelectionLabel {
            targets,
            field: Default::default(),
            edits: Vec::new(),
        },
    };
    let plan = w.flow.plan(command)?;
    let flow = w.flow.clone();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(height)), |window, cx| {
        pin(cx, light);
        cx.new(|cx| {
            BulkDialog::new(
                kind,
                flow,
                plan,
                sample,
                WeakEntity::new_invalid(),
                window,
                cx,
            )
        })
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, window, cx| {
        let dialog = view.downcast::<BulkDialog>().expect("the root view");
        dialog.update(cx, |dialog, cx| drive(dialog, window, cx));
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn editor(light: bool) -> anyhow::Result<RgbaImage> {
    let w = world();
    let targets = (0..3)
        .map(|i| pod_ref(&w.cluster, "shop", &format!("web-{i}")))
        .collect();
    let sample = BulkSample {
        name: "web-0".into(),
        labels: vec![
            ("app".into(), "web".into()),
            ("pod-template-hash".into(), "5d8f".into()),
            ("tier".into(), "frontend".into()),
        ],
        annotations: Vec::new(),
    };
    shot(
        &w,
        BulkKind::Label,
        targets,
        sample,
        EDITOR_HEIGHT,
        light,
        |dialog, window, cx| {
            let editor = dialog.editor_mut().expect("the editor");
            editor.fill(0, "tier", "backend", window, cx);
            editor.add_row(window, cx);
            editor.fill(1, "team", "payments", window, cx);
            editor.add_row(window, cx);
            editor.fill(2, "pod-template-hash", "", window, cx);
            editor.toggle_remove(2);
        },
    )
}

fn delete(light: bool) -> anyhow::Result<RgbaImage> {
    let w = world();
    let mut targets: Vec<ResourceRef> = (0..3)
        .map(|i| pod_ref(&w.cluster, "shop", &format!("web-{i}")))
        .collect();
    targets.push(pod_ref(&w.cluster, "kube-system", "coredns-1"));
    targets.push(ResourceRef::cluster_scoped(
        w.cluster.clone(),
        Gvk::new("", "v1", "Namespace"),
        "staging",
    ));
    shot(
        &w,
        BulkKind::Delete,
        targets,
        BulkSample::default(),
        DELETE_HEIGHT,
        light,
        |dialog, window, cx| dialog.type_text("kind-ox", window, cx),
    )
}

fn results(light: bool) -> anyhow::Result<RgbaImage> {
    let w = world();
    let ports = w.connector.ports_for(&w.cluster);
    // web-0 and web-1 go; web-2 is refused; web-3 was already gone; web-4 goes.
    let script = ports.resources.script();
    for _ in 0..4 {
        script.delete.push_ok(DeleteOutcome::Deleted);
    }
    script.delete.push_err(OxiError::forbidden(
        "pods \"web-2\" is forbidden: cannot delete",
    ));
    script
        .delete
        .push_err(OxiError::not_found("pods \"web-3\" not found"));
    script.delete.push_ok(DeleteOutcome::Deleted);
    script.delete.push_ok(DeleteOutcome::Deleted);
    let targets = (0..5)
        .map(|i| pod_ref(&w.cluster, "shop", &format!("web-{i}")))
        .collect();
    shot(
        &w,
        BulkKind::Delete,
        targets,
        BulkSample::default(),
        RESULTS_HEIGHT,
        light,
        |dialog, _, cx| dialog.confirm(cx),
    )
}

pub(crate) fn run() -> anyhow::Result<()> {
    check(
        "bulk_label_editor_dark",
        editor(false)?,
        WIDTH,
        EDITOR_HEIGHT,
    )?;
    check(
        "bulk_label_editor_light",
        editor(true)?,
        WIDTH,
        EDITOR_HEIGHT,
    )?;
    check(
        "bulk_delete_confirm_dark",
        delete(false)?,
        WIDTH,
        DELETE_HEIGHT,
    )?;
    check(
        "bulk_delete_confirm_light",
        delete(true)?,
        WIDTH,
        DELETE_HEIGHT,
    )?;
    check("bulk_results_dark", results(false)?, WIDTH, RESULTS_HEIGHT)?;
    check("bulk_results_light", results(true)?, WIDTH, RESULTS_HEIGHT)
}
