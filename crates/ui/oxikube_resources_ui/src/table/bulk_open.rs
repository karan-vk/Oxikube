//! Bulk actions on the table's selection (E12-S19): opening the bulk dialog for the selected rows.
//!
//! [`ResourceTable::run_action`] routes here when the action is a bulk one ([`BulkKind::of_action`]):
//! `Delete` or `Restart` with several rows selected, `Edit labels…`, and the palette's
//! `selection::*` commands. The dialog, not the table, plans, asks and runs.

use gpui::{Context, Window};
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
    /// selected row. Reads the rows already in memory.
    fn bulk_sample(&self, targets: &[ResourceRef], cx: &gpui::App) -> BulkSample {
        let wanted = |name: &str, namespace: Option<&str>| {
            targets
                .iter()
                .any(|t| &*t.name == name && t.namespace() == namespace)
        };
        let cursor = self.cursor_row(cx);
        self.read_rows(cx, |rows| {
            let from = |ix: usize| {
                let resource = rows.row(ix)?.resource()?;
                wanted(resource.name(), resource.namespace()).then(|| resource.clone())
            };
            let resource = cursor
                .and_then(from)
                .or_else(|| (0..rows.rows().len()).find_map(from));
            let Some(resource) = resource else {
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
