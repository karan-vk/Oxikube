//! The table's columns: the provider that reads them, the saved layout (read once, in the
//! background; defaults until then) and saving it after each change.

use std::sync::Arc;

use gpui::{App, AppContext as _, Context};
use oxikube_app::store::FeedKind;
use oxikube_app::{ColumnProvider, TableColumns};
use oxikube_domain::Capabilities;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kinds::ResourceKind;
use oxikube_ports::TableSource;

use super::layout::ColumnLayout;
use super::prefs::{ColumnPrefs, ColumnPrefsStore, PrefsWriter};
use super::view::{ResourceTable, ResourceTableDeps};

impl ResourceTable {
    pub(super) fn load_prefs(&mut self, cx: &mut Context<Self>) {
        let store = match ColumnPrefsStore::new(self.deps.state.clone(), &self.kind.gvk) {
            Ok(store) => store,
            Err(error) => {
                tracing::warn!(%error, kind = %self.kind.gvk, "column layout is not saved");
                self.prefs_loaded = true;
                return;
            }
        };
        self.writer = Some(PrefsWriter::spawn(store.clone(), cx));
        let load = cx.background_spawn(async move { store.load().await });
        self.prefs_task = Some(cx.spawn(async move |this, cx| {
            let prefs = match load.await {
                Ok(prefs) => prefs.unwrap_or_default(),
                Err(error) => {
                    tracing::warn!(%error, "reading the column layout failed: defaults apply");
                    ColumnPrefs::default()
                }
            };
            this.update(cx, |view, cx| view.apply_prefs(&prefs, cx))
                .ok();
        }));
    }

    /// Applies the saved layout (once, when it has been read).
    fn apply_prefs(&mut self, prefs: &ColumnPrefs, cx: &mut Context<Self>) {
        self.prefs_loaded = true;
        let caps = session_capabilities(&self.deps, &self.cluster);
        let gvk = self.kind.gvk.clone();
        self.table.update(cx, |d| {
            d.layout = ColumnLayout::new(d.provider.columns(&gvk, caps), prefs);
        });
        self.table.refresh(cx);
        self.apply_sort(cx);
    }

    /// Saves the layout as it is now (after the saved one was read).
    pub(super) fn save_prefs(&self, cx: &App) {
        if !self.prefs_loaded {
            return;
        }
        if let Some(writer) = &self.writer {
            writer.save(self.table.read(cx, |d| d.layout.prefs()));
        }
    }

    /// Replaces the column provider (a Table feed delivered its columns), keeping the user's
    /// layout.
    pub(super) fn set_provider(
        &mut self,
        provider: Arc<dyn ColumnProvider>,
        cx: &mut Context<Self>,
    ) {
        self.relayout(Some(provider), cx);
    }

    /// Rebuilds the layout over the provider's columns as they are now (the provider is first
    /// replaced by `provider`, if given), keeping the user's choices, including those for
    /// columns that are absent now and may come back. Hands the resulting sort to the
    /// subscription: a sort column that went falls back to the default order, one that came
    /// back sorts again.
    pub(super) fn relayout(
        &mut self,
        provider: Option<Arc<dyn ColumnProvider>>,
        cx: &mut Context<Self>,
    ) {
        let caps = session_capabilities(&self.deps, &self.cluster);
        let gvk = self.kind.gvk.clone();
        let changed = self.table.update_quiet(cx, |d| {
            let swapped = provider.is_some();
            if let Some(provider) = provider {
                d.provider = provider;
            }
            let next = ColumnLayout::new(d.provider.columns(&gvk, caps), &d.layout.prefs());
            let changed = swapped || next != d.layout;
            d.layout = next;
            changed
        });
        if changed {
            self.table.refresh(cx);
            self.apply_sort(cx);
        }
    }
}

/// The provider a new table starts with: the core catalogue, or for a kind on a Table feed the
/// generic Name / Namespace / Age columns until the feed's own columns arrive.
pub(super) fn initial_provider(
    cluster: &ClusterId,
    kind: &ResourceKind,
    deps: &ResourceTableDeps,
) -> Arc<dyn ColumnProvider> {
    let on_table_feed = deps
        .sessions
        .get(cluster)
        .and_then(|session| deps.stores.for_session(&session))
        .is_some_and(|store| store.plan(&kind.gvk).kind == FeedKind::Table);
    if on_table_feed {
        Arc::new(TableColumns::new(&[], TableSource::Objects, kind.scope()))
    } else {
        deps.columns.clone()
    }
}

/// What the cluster's session serves (metrics columns come and go with it); none while it has no
/// session.
pub(super) fn session_capabilities(deps: &ResourceTableDeps, cluster: &ClusterId) -> Capabilities {
    deps.sessions
        .get(cluster)
        .map(|s| s.capabilities())
        .unwrap_or_default()
}
