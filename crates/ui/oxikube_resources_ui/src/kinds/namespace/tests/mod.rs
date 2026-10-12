//! The Namespaces kind in the real table, dialogs and detail over testkit fakes (a real
//! `CommandBus` and `MutationGuard`; no cluster, no threads).

mod allocation;
mod create;
mod delete;
mod keys;
mod panel;
mod table;

use gpui::{Entity, TestAppContext};
use jiff::Timestamp;
use oxikube_app::ColumnId;
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use serde_json::{Value, json};

use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};
use crate::table::tests::p;

/// The Namespace kind as discovery serves it (cluster-scoped).
pub(super) fn namespaces_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Namespace"),
        preferred: true,
        plural: "namespaces".into(),
        singular: "namespace".into(),
        short_names: vec!["ns".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "create", "delete"]),
        namespaced: false,
    }
}

/// A namespace `name`; `extra` is merged into its metadata.
pub(super) fn ns(name: &str, extra: Value) -> Resource {
    let mut value = json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": name,
            "resourceVersion": "1",
            "creationTimestamp": "2026-01-01T00:00:00Z",
        },
        "spec": { "finalizers": ["kubernetes"] },
        "status": { "phase": "Active" },
    });
    if let Some(extra) = extra.as_object() {
        for (key, v) in extra {
            value["metadata"][key] = v.clone();
        }
    }
    Resource::from_json(value).expect("a namespace")
}

pub(super) fn ns_ref(name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Namespace"), name)
}

/// A window with the built-in kinds registered, a connected cluster holding `namespaces` and
/// `pods`, and the Namespaces table open.
pub(super) struct Env {
    pub f: Fixture,
    pub table: Entity<ResourceTable>,
}

impl Env {
    pub(super) fn new(
        cx: &mut TestAppContext,
        namespaces: impl IntoIterator<Item = Resource>,
        pods: impl IntoIterator<Item = Resource>,
    ) -> Self {
        let mut f = Fixture::with_actions(cx);
        f.vcx.update(|_, cx| crate::kinds::init(cx));
        let ports = f.ports();
        for namespace in namespaces {
            ports.resources.insert(namespace);
        }
        f.connect_with(pods);
        let table = f.open(namespaces_kind());
        Self { f, table }
    }

    /// The namespaces and a few pods: `team-a` holds two, `team-b` one.
    pub(super) fn standard(cx: &mut TestAppContext) -> Self {
        Self::new(
            cx,
            [
                ns("kube-system", json!({})),
                ns("team-a", json!({ "labels": { "team": "a" } })),
                ns("team-b", json!({})),
            ],
            [
                p("team-a", "web-0", "1"),
                p("team-a", "web-1", "1"),
                p("team-b", "db-0", "1"),
            ],
        )
    }

    /// The cell text of row `row` in column `column`.
    pub(super) fn cell(&mut self, row: usize, column: &str) -> String {
        let table = self.table.clone();
        self.f.vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                let object = d.row(row).expect("a row");
                d.provider()
                    .cell(object, &ColumnId::new(column), Timestamp::now())
                    .display()
                    .to_owned()
            })
        })
    }

    /// Moves the cursor to the row named `name`.
    pub(super) fn cursor_to(&mut self, name: &str) {
        let at = self
            .f
            .names(&self.table)
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("no row {name}"));
        let table = self.table.clone();
        self.f.keys(&table, &"down ".repeat(at + 1));
    }

    pub(super) fn keys(&mut self, keys: &str) {
        let table = self.table.clone();
        self.f.keys(&table, keys);
    }
}
