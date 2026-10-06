//! Following the session: the rules reviews, discovery, and rebuilding the rows.

use futures::StreamExt as _;
use gpui::{Context, WeakEntity};
use oxikube_app::{
    AccessOutcome, CustomResourceGroup, SessionChange, sidebar::discover_custom_resources,
    sidebar::review_access,
};
use oxikube_domain::OxiResult;
use oxikube_runtime::spawn_kube;

use super::SidebarPanel;
use crate::sidebar::registry::SidebarRegistry;
use crate::sidebar::rows::{AccessState, RowInputs, build_rows};

impl SidebarPanel {
    /// Starts everything the panel follows: the registry, the saved state, the session.
    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        // A section registered after the panel exists (a feature's `init`, a test) appears.
        self._subscriptions
            .push(cx.observe_global::<SidebarRegistry>(|this, cx| this.rebuild(cx)));
        self.load_saved(cx);
        self.start_counts(cx);

        // Subscribe before reading the session, so no update falls between the two.
        let mut updates = self.deps.sessions.subscribe();
        let cluster = self.cluster.clone();
        self._watch_session = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(item) = updates.next().await {
                let alive = this.update(cx, |this, cx| match item {
                    Ok(update) if update.cluster == cluster => this.on_change(update.change, cx),
                    Ok(_) => {}
                    // Missed some: look at the session as it is now.
                    Err(_) => this.resync(cx),
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        self.resync(cx);
    }

    /// Reads the session as it is: build the rows, and review its access when it is connected.
    fn resync(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
        let connected = self
            .deps
            .sessions
            .get(&self.cluster)
            .is_some_and(|s| s.is_connected());
        if connected {
            self.refresh_access(cx);
            self.refresh_custom_resources(cx);
        }
    }

    fn on_change(&mut self, change: SessionChange, cx: &mut Context<Self>) {
        match change {
            // A first connect or a reconnect: review again. Flaps between Ready and Degraded
            // are the same connection.
            SessionChange::StateChanged { from, state }
                if state.phase().is_connected() && !from.is_connected() =>
            {
                self.rebuild(cx);
                self.refresh_access(cx);
                self.refresh_custom_resources(cx);
            }
            // Which namespaces are selected decides which rules apply.
            SessionChange::NamespaceChanged(_) => {
                self.refresh_access(cx);
                self.rescope_counts(cx);
            }
            // Integrations' sections depend on what the cluster offers.
            SessionChange::CapabilitiesChanged(_) => self.rebuild(cx),
            _ => {}
        }
    }

    /// Reviews the user's access for the selected namespaces, on the Tokio bridge. A review in
    /// flight is cancelled by the newer one; the rows keep showing the previous answer until it
    /// lands.
    pub(super) fn refresh_access(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.deps.sessions.get(&self.cluster) else {
            return;
        };
        let Some(access) = session.access() else {
            return;
        };
        let selection = session.namespace_selection().clone();
        self.review_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let outcome = spawn_kube(cx, async move {
                review_access(access.as_ref(), &selection).await
            })
            .await
            .unwrap_or_else(|error| AccessOutcome::Failed {
                reason: error.to_string(),
            });
            this.update(cx, |this, cx| this.apply_review(outcome, cx))
                .ok();
        }));
    }

    /// Lists the cluster's custom resource kinds, on the Tokio bridge.
    pub(super) fn refresh_custom_resources(&mut self, cx: &mut Context<Self>) {
        let Some(discovery) = self
            .deps
            .sessions
            .get(&self.cluster)
            .and_then(|s| s.discovery())
        else {
            return;
        };
        self.discovery_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let groups = spawn_kube(cx, async move {
                discover_custom_resources(discovery.as_ref()).await
            })
            .await
            .map_err(oxikube_domain::OxiError::from)
            .and_then(|found| found);
            this.update(cx, |this, cx| this.apply_custom(groups, cx))
                .ok();
        }));
    }

    pub(super) fn apply_review(&mut self, outcome: AccessOutcome, cx: &mut Context<Self>) {
        self.access = AccessState::Known(outcome);
        self.rebuild(cx);
    }

    pub(super) fn apply_custom(
        &mut self,
        groups: OxiResult<Vec<CustomResourceGroup>>,
        cx: &mut Context<Self>,
    ) {
        match groups {
            Ok(groups) => self.custom = Some(groups),
            // Without discovery there is nothing to list: the section stays hidden.
            Err(error) => {
                tracing::warn!(%error, cluster = %self.cluster, "listing custom resources failed")
            }
        }
        self.rebuild(cx);
    }

    /// Reads the saved open and closed groups, off the UI thread.
    fn load_saved(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        self.load_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            match store.load().await {
                Ok(Some(saved)) => {
                    this.update(cx, |this, cx| {
                        // What the user already toggled this session wins over what was saved.
                        for (id, open) in saved.open {
                            this.open.entry(id).or_insert(open);
                        }
                        this.rebuild(cx);
                    })
                    .ok();
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "reading the sidebar state failed"),
            }
        }));
    }

    /// Rebuilds the rows from the registry, the integrations, discovery, the review and the open
    /// groups; redraws only when they changed.
    pub(super) fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.sections = SidebarRegistry::sections(cx);
        let capabilities = self
            .deps
            .sessions
            .get(&self.cluster)
            .map(|s| s.capabilities())
            .unwrap_or_default();
        self.integrations = self.deps.integrations.sidebar_sections(capabilities);
        self.plan_counts();
        let mut rows = build_rows(&RowInputs {
            sections: &self.sections,
            integrations: &self.integrations,
            custom: self.custom.as_deref(),
            access: &self.access,
            open: &self.open,
        });
        self.apply_count_states(&mut rows);
        if rows != self.rows {
            self.rows = rows;
            self.fix_highlight();
            cx.notify();
        }
        self.sync_counts(cx);
    }
}
