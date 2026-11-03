//! The per-cluster scope on the page: switching scope, the cluster list, the catalog, and the
//! precedence strip under each field.

use futures::StreamExt as _;
use gpui::{Context, Window};
use oxikube_app::{CatalogEntry, ClusterCatalog};
use oxikube_domain::ids::ClusterId;
use oxikube_runtime::spawn_kube;
use oxikube_settings::SettingsStore;

use super::choice::{ClusterChoice, ClusterRowText, choices};
use super::model::cluster_page_model;
use super::state::{Parked, Scope};
use crate::page::{SearchIndex, SettingsPage, SettingsPageModel};

impl SettingsPage {
    /// Which settings the page edits: all of them, or one cluster's.
    pub fn scope(&self) -> Scope {
        self.tab.scope
    }

    /// The rows of the cluster list.
    pub fn cluster_choices(&self) -> &[ClusterChoice] {
        &self.tab.choices
    }

    /// The cluster the per-cluster tab edits (`None` on the global scope, or before one is
    /// picked).
    pub fn selected_cluster(&self) -> Option<&ClusterId> {
        self.cluster.as_ref()
    }

    /// Lists the clusters of `catalog` on the per-cluster tab and follows its changes. The load
    /// runs on the Kubernetes bridge; the page redraws once when it lands.
    pub fn set_catalog(&mut self, catalog: ClusterCatalog, cx: &mut Context<Self>) {
        let mut changes = catalog.changes();
        self.tab.catalog = Some(catalog);
        self.reload_clusters(cx);
        self.tab.watch = Some(cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this
                    .update(cx, |this, cx| this.reload_clusters(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn reload_clusters(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.tab.catalog.clone() else {
            return;
        };
        self.tab.load = Some(cx.spawn(async move |this, cx| {
            let loaded = spawn_kube(&*cx, async move { catalog.load().await }).await;
            match loaded {
                Ok(Ok(mut entries)) => {
                    entries.sort_by(CatalogEntry::cmp_default);
                    this.update(cx, |this, cx| this.set_catalog_entries(entries, cx))
                        .ok();
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the cluster catalog could not be read for the settings page");
                }
                Err(error) => {
                    tracing::warn!(%error, "the cluster catalog load did not finish");
                }
            }
        }));
    }

    /// Sets the clusters the list offers to the catalog's `entries` (plus the file's blocks they
    /// do not cover). Keeps the picked cluster when it is still listed, else picks the first.
    pub fn set_catalog_entries(&mut self, entries: Vec<CatalogEntry>, cx: &mut Context<Self>) {
        self.tab.entries = entries;
        self.refresh_choices(cx);
    }

    /// Rebuilds the list from the entries and the file.
    fn refresh_choices(&mut self, cx: &mut Context<Self>) {
        let store = cx.try_global::<SettingsStore>();
        self.tab.choices = choices(&self.tab.entries, store);
        self.tab.rows = self
            .tab
            .choices
            .iter()
            .map(|choice| choice.row_text(store))
            .collect();
        if self.tab.scope == Scope::Cluster {
            let listed = |id: &ClusterId| self.tab.choices.iter().any(|row| row.id == *id);
            if !self.cluster.as_ref().is_some_and(listed) {
                let first = self.tab.choices.first().map(|row| row.id.clone());
                self.pick(first);
            }
        }
        cx.notify();
    }

    /// The file changed: the rows' display names and override counts follow it. Nothing to do
    /// on the global scope (the rows are rebuilt when the tab is shown).
    pub(in crate::page) fn refresh_cluster_rows(&mut self, cx: &Context<Self>) {
        if self.tab.scope != Scope::Cluster {
            return;
        }
        let store = cx.try_global::<SettingsStore>();
        self.tab.rows = self
            .tab
            .choices
            .iter()
            .map(|choice| choice.row_text(store))
            .collect();
    }

    /// What each row of the cluster list shows, in the list's order.
    pub fn cluster_rows(&self) -> &[ClusterRowText] {
        &self.tab.rows
    }

    /// Shows the all-settings page or the per-cluster tab. Clears the search; the per-cluster
    /// tab opens on the cluster picked last, else the first in the list.
    pub fn set_scope(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab.scope == scope {
            return;
        }
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.query_text.clear();
        self.hits.clear();
        self.highlighted = None;
        let other = self
            .tab
            .parked
            .take()
            .unwrap_or_else(|| self.build_scope(scope, cx));
        let current = Parked {
            model: std::mem::replace(&mut self.model, other.model),
            index: std::mem::replace(&mut self.index, other.index),
            selected: std::mem::replace(&mut self.selected, other.selected),
        };
        self.tab.parked = Some(current);
        self.tab.scope = scope;
        match scope {
            Scope::Global => {
                self.tab.last = self.cluster.take().or(self.tab.last.take());
                self.controls.forget_editors();
            }
            Scope::Cluster => {
                self.refresh_choices(cx);
                let wanted = self
                    .tab
                    .last
                    .clone()
                    .filter(|id| self.tab.choices.iter().any(|row| row.id == *id))
                    .or_else(|| self.tab.choices.first().map(|row| row.id.clone()));
                self.pick(wanted);
            }
        }
        self.content_scroll.scroll_to_item(0);
        cx.notify();
    }

    /// The model and index of `scope`, built from the store's schema.
    fn build_scope(&self, scope: Scope, cx: &Context<Self>) -> Parked {
        let schema = cx
            .try_global::<SettingsStore>()
            .map(SettingsStore::json_schema);
        let model = match (&schema, scope) {
            (None, _) => SettingsPageModel::default(),
            (Some(schema), Scope::Cluster) => cluster_page_model(schema),
            (Some(schema), Scope::Global) => SettingsPageModel::from_schema(schema),
        };
        let index = SearchIndex::new(&model);
        Parked {
            model,
            index,
            selected: 0,
        }
    }

    /// Edits `cluster` on the per-cluster tab. Ignored when the list does not offer it.
    pub fn select_cluster(&mut self, cluster: &ClusterId, cx: &mut Context<Self>) {
        if self.tab.scope != Scope::Cluster
            || !self.tab.choices.iter().any(|row| row.id == *cluster)
        {
            return;
        }
        self.pick(Some(cluster.clone()));
        cx.notify();
    }

    /// Points the controls at `cluster`: their inputs are rebuilt from its values.
    fn pick(&mut self, cluster: Option<ClusterId>) {
        if self.cluster == cluster {
            return;
        }
        self.tab.last = cluster.clone().or(self.tab.last.take());
        self.cluster = cluster;
        self.controls.forget_editors();
    }

    /// How many elements precede the section header in the content column.
    pub(in crate::page) fn header_offset(&self) -> usize {
        usize::from(self.tab.scope == Scope::Cluster)
    }
}
