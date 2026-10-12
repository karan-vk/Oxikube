//! Bulk actions on the table's selection (E12-S19): opening the bulk dialog for the selected rows.
//!
//! [`ResourceTable::run_action`] routes here when the action is a bulk one ([`BulkKind::of_action`]):
//! `Delete` or `Restart` with several rows selected, `Edit labels…`, and the palette's
//! `selection::*` commands. The dialog, not the table, plans, asks and runs.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{Context, Window};
use oxikube_app::store::StoreObject;
use oxikube_domain::Resource;
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::Toast;

use super::view::ResourceTable;
use crate::actions::ResourceActions;
use crate::bulk::{BulkKind, BulkSample};

impl ResourceTable {
    /// Opens the bulk dialog for `bulk` over `targets`, unless the single-object action it follows
    /// is not offered for this kind (a toast says why) or is disabled (read-only: its reason).
    pub(super) fn run_bulk(
        &mut self,
        actions: &ResourceActions,
        bulk: BulkKind,
        targets: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let gate = bulk.row_action(&self.kind);
        let selected = targets.len();
        let entries = actions.entries(&self.cluster, &self.kind, selected);
        let Some(entry) = entries.into_iter().find(|entry| entry.command() == gate) else {
            self.say_unavailable(actions, gate, selected, cx);
            return false;
        };
        if let Some(reason) = entry.reason() {
            self.toast(
                Toast::warning(reason).key(format!("action-disabled:{gate}")),
                cx,
            );
            return false;
        }
        self.begin_bulk(actions, bulk, targets, window, cx);
        true
    }

    /// Opens the bulk dialog for `kind` over `targets`.
    pub(super) fn begin_bulk(
        &mut self,
        actions: &ResourceActions,
        kind: BulkKind,
        targets: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.clone() else {
            tracing::warn!("a resource table without a workspace cannot open the bulk dialog");
            return;
        };
        let sample = self.bulk_sample(&targets, cx);
        if let Err(error) = actions
            .bulk_flow(workspace)
            .begin(kind, targets, sample, window, cx)
        {
            self.toast(Toast::error(error.to_string()), cx);
        }
    }

    /// The object the label editor previews: the cursor row when it is selected, else the first
    /// selected row. Reads the rows already in memory (one pass, not a scan of the selection per
    /// row).
    fn bulk_sample(&self, targets: &[ResourceRef], cx: &gpui::App) -> BulkSample {
        let cursor = self.cursor_row(cx);
        self.read_rows(cx, |rows| {
            let Some(resource) = sample_resource(rows.rows(), cursor, targets) else {
                return BulkSample::default();
            };
            let pairs = |map: &oxikube_domain::resource::StrMap| -> Vec<(String, String)> {
                map.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            };
            BulkSample {
                name: resource.name().to_owned(),
                labels: pairs(&resource.meta.labels),
                annotations: pairs(&resource.meta.annotations),
            }
        })
    }
}

/// The row the label editor previews among `rows`: the one at `cursor` when it is one of
/// `targets`, else the first row that is. One hash set of the targets, so the cost is the rows
/// plus the targets, however they are laid out.
fn sample_resource<'a>(
    rows: &'a [Arc<StoreObject>],
    cursor: Option<usize>,
    targets: &[ResourceRef],
) -> Option<&'a Resource> {
    let wanted: HashSet<(&str, Option<&str>)> =
        targets.iter().map(|t| (&*t.name, t.namespace())).collect();
    let from = |ix: usize| {
        let resource = rows.get(ix)?.resource()?;
        wanted
            .contains(&(resource.name(), resource.namespace()))
            .then_some(resource)
    };
    cursor
        .and_then(from)
        .or_else(|| (0..rows.len()).find_map(from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::tests::{p, rows};

    fn target(name: &str) -> ResourceRef {
        let resource = p("x", name, "1");
        ResourceRef::namespaced(
            oxikube_domain::ids::ClusterId::new("kubeconfig", &"ctx".into()),
            resource.kind,
            "x",
            name,
        )
    }

    fn name_of(found: Option<&Resource>) -> Option<&str> {
        found.map(Resource::name)
    }

    #[test]
    fn the_cursor_row_is_the_sample_when_it_is_selected() {
        let list = rows(&["a", "b", "c", "d"]);
        let targets = [target("b"), target("d")];
        assert_eq!(
            name_of(sample_resource(&list, Some(3), &targets)),
            Some("d")
        );
    }

    #[test]
    fn an_unselected_cursor_row_falls_back_to_the_first_selected_row() {
        let list = rows(&["a", "b", "c", "d"]);
        let targets = [target("d"), target("c")];
        assert_eq!(
            name_of(sample_resource(&list, Some(0), &targets)),
            Some("c")
        );
        assert_eq!(name_of(sample_resource(&list, None, &targets)), Some("c"));
        assert_eq!(
            name_of(sample_resource(&list, Some(99), &targets)),
            Some("c")
        );
    }

    #[test]
    fn no_selected_row_in_memory_is_no_sample() {
        let list = rows(&["a", "b"]);
        assert_eq!(
            name_of(sample_resource(&list, Some(0), &[target("z")])),
            None
        );
    }

    #[test]
    fn a_large_selection_below_an_unselected_cursor_is_found_in_one_pass() {
        // 5k unselected rows above 5k selected ones: the shape that was rows x targets.
        let names: Vec<String> = (0..10_000).map(|i| format!("pod-{i:05}")).collect();
        let list = rows(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let targets: Vec<ResourceRef> = names[5_000..].iter().map(|n| target(n)).collect();
        let started = std::time::Instant::now();
        let found = sample_resource(&list, Some(0), &targets);
        assert_eq!(name_of(found), Some("pod-05000"));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "took {:?}",
            started.elapsed()
        );
    }
}
