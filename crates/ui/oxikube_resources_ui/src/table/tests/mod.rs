//! Tests of the resource table: the pure models (selection, layout, prefs) as plain tests, the
//! view as `#[gpui::test]`s over testkit fakes (a fake connector and a real session manager and
//! store, a fake state port, a recording dispatcher). No cluster, no disk, no threads.

mod coalesce;
pub(crate) mod fixture;
mod layout;
mod prefs;
mod selection;
mod view_columns;
mod view_filter;
mod view_filter_keys;
mod view_filter_saved;
mod view_rows;
mod view_select;
mod view_states;
mod views;

use std::sync::Arc;

use oxikube_app::store::{ObjectKey, StoreObject};
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_testkit::pod;

/// The pods kind as discovery serves it.
pub(crate) fn pods_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Pod"),
        preferred: true,
        plural: "pods".into(),
        singular: "pod".into(),
        short_names: vec!["po".into()],
        categories: vec!["all".into()],
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: true,
    }
}

/// Pod `ns/name`, version `rv`.
pub(crate) fn p(ns: &str, name: &str, rv: &str) -> Resource {
    let mut r = pod().namespace(ns).name(name).build();
    r.meta.resource_version = Some(rv.into());
    r
}

/// Rows of pods named `names` in namespace `x`.
pub(super) fn rows(names: &[&str]) -> Vec<Arc<StoreObject>> {
    names
        .iter()
        .map(|n| Arc::new(StoreObject::Resource(p("x", n, "1"))))
        .collect()
}

pub(super) fn key(name: &str) -> ObjectKey {
    ObjectKey::new(Some("x"), name)
}
