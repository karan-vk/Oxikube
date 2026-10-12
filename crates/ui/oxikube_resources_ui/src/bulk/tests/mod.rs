//! The bulk dialog (E12-S19) in a real table over the real bus and guard on testkit fakes:
//! `#[gpui::test]`s of delete, restart and the label editor for a selection, the confirmation, the
//! results and the retry.

mod delete;
mod label;
mod restart;
mod results;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::audit::AuditRecord;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use serde_json::json;

use super::BulkDialog;
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

/// The platform's select-all chord (the keymaps bind `cmd-a` on macOS, `ctrl-a` elsewhere).
pub(super) const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};

/// A kind as discovery serves it.
pub(super) fn kind(group: &str, kind: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", kind),
        preferred: true,
        plural: plural.into(),
        singular: kind.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete", "patch"]),
        namespaced: true,
    }
}

pub(super) fn deployments() -> ResourceKind {
    kind("apps", "Deployment", "deployments")
}

pub(super) fn replicasets() -> ResourceKind {
    kind("apps", "ReplicaSet", "replicasets")
}

/// Deployment `shop/<name>`.
pub(super) fn deployment(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": name, "namespace": "shop", "uid": format!("{name}-uid"),
                     "resourceVersion": "3", "creationTimestamp": "2026-01-01T00:00:00Z",
                     "labels": {"app": name}},
        "spec": {"replicas": 1, "selector": {"matchLabels": {"app": name}},
                 "template": {"metadata": {"labels": {"app": name}},
                              "spec": {"containers": [{"name": "web", "image": "web:1"}]}}},
        "status": {"replicas": 1, "readyReplicas": 1, "updatedReplicas": 1, "availableReplicas": 1},
    }))
    .expect("a deployment")
}

/// A fixture with the workload handlers and the deployments and pods kinds served; `objects` are
/// in the cluster, which is connected.
pub(super) fn window(
    cx: &mut TestAppContext,
    objects: impl IntoIterator<Item = Resource>,
) -> Fixture {
    let mut f = Fixture::with_workloads(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.ports().discovery.set_kinds([
        deployments(),
        replicasets(),
        kind("", "Pod", "pods"),
        kind("", "Namespace", "namespaces"),
    ]);
    for object in objects {
        f.ports().resources.insert(object);
    }
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    f
}

/// The bulk dialog open in the cluster tab's workspace, if any.
pub(super) fn dialog(f: &mut Fixture) -> Option<Entity<BulkDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let layer = tab.read(cx).workspace().read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<BulkDialog>()
    })
}

/// Whether the part with debug selector `selector` is drawn.
pub(super) fn drawn(f: &mut Fixture, selector: &'static str) -> bool {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.vcx.debug_bounds(selector).is_some()
}

/// Runs `run` on the dialog and settles.
pub(super) fn with_dialog<R>(
    f: &mut Fixture,
    dialog: &Entity<BulkDialog>,
    run: impl FnOnce(&mut BulkDialog, &mut gpui::Window, &mut gpui::Context<BulkDialog>) -> R,
) -> R {
    let dialog = dialog.clone();
    let out = f
        .vcx
        .update(|window, cx| dialog.update(cx, |dialog, cx| run(dialog, window, cx)));
    f.settle();
    out
}

/// Selects every row of `table` and returns it.
pub(super) fn select_all(f: &mut Fixture, table: &Entity<ResourceTable>) {
    f.keys(table, SELECT_ALL);
}

/// The audit records whose command is `cmd`.
pub(super) fn audit_of(f: &Fixture, cmd: &str) -> Vec<AuditRecord> {
    f.state
        .audit_log()
        .into_iter()
        .filter(|record| &*record.cmd == cmd)
        .collect()
}

/// Namespaced pods `x/web-<i>`.
pub(super) fn pods(n: usize) -> impl Iterator<Item = Resource> {
    (0..n).map(|i| crate::table::tests::p("x", &format!("web-{i}"), "1"))
}
