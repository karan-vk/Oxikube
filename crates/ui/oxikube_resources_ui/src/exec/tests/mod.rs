//! "Shell" and "Attach" on a pod's row, key and detail header: the container is chosen (a single
//! one, the default, or a picker) before the command is dispatched, a read-only cluster blocks
//! them unless it allows them, and nothing is sent when the picker is cancelled.

mod detail;
mod menu;
mod picker;

use gpui::Entity;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use serde_json::{Value, json};

use crate::exec::ContainerPicker;
use crate::table::tests::fixture::{Fixture, cluster};

/// `ns/name` as a pod with `containers` (all running) in its spec and status.
pub(super) fn pod_with(ns: &str, name: &str, containers: &[&str]) -> Resource {
    let specs: Vec<Value> = containers
        .iter()
        .map(|c| json!({"name": c, "image": "busybox"}))
        .collect();
    let statuses: Vec<Value> = containers
        .iter()
        .map(|c| {
            json!({"name": c, "ready": true, "restartCount": 0, "image": "busybox",
                   "state": {"running": {"startedAt": "2026-01-01T00:00:00Z"}}})
        })
        .collect();
    let mut resource = oxikube_testkit::pod().namespace(ns).name(name).build();
    let mut json = resource.json.clone();
    json["spec"]["containers"] = Value::Array(specs);
    json["status"]["containerStatuses"] = Value::Array(statuses);
    json["metadata"]["resourceVersion"] = json!("1");
    resource = Resource::from_json(json).expect("a pod");
    resource
}

/// The pod `x/web-0` as a reference.
pub(super) fn web_ref() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "x", "web-0")
}

/// The picker open in the cluster tab's workspace, if any.
pub(super) fn picker(f: &mut Fixture) -> Option<Entity<ContainerPicker>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let workspace = tab.read(cx).workspace().clone();
        let layer = workspace.read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<ContainerPicker>()
    })
}

/// Only the exec commands among what the dispatcher saw.
pub(super) fn sent_exec(f: &Fixture) -> Vec<Command> {
    f.dispatcher
        .sent()
        .into_iter()
        .filter(Command::is_exec)
        .collect()
}
