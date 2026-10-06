//! [`SessionRestorer`]: reopen the saved session, then connect what the settings ask for.

use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_ports::{ClusterSourcePort, ExecInteractivity};

use super::config::{RestoreConfig, RestoreConnect};
use super::plan::RestorePlan;
use super::report::{ConnectOutcome, RestoreReport};
use super::saved::ClusterTabsStore;
use crate::session::ClusterSessionManager;
use crate::session::namespaces::NamespaceService;

/// Restores the previous session: [`prepare`](Self::prepare) reopens the saved clusters (cheap:
/// nothing connects), [`connect`](Self::connect) brings up the ones that should be live at launch.
///
/// Two calls, not one, so the UI can show the restored tabs as placeholders between them. Both
/// are plain async: run them on the Tokio bridge (`oxikube_runtime::spawn_kube`), and only after
/// the first frame (the window decides when).
///
/// Cheap to clone.
#[derive(Clone)]
pub struct SessionRestorer {
    sessions: ClusterSessionManager,
    namespaces: NamespaceService,
    source: Arc<dyn ClusterSourcePort>,
    store: ClusterTabsStore,
    config: RestoreConfig,
}

impl SessionRestorer {
    /// A restorer over `sessions`. `store` is the window's saved tabs; `source` is the catalog the
    /// saved clusters are matched against; `namespaces` restores each cluster's selection.
    pub fn new(
        sessions: ClusterSessionManager,
        namespaces: NamespaceService,
        source: Arc<dyn ClusterSourcePort>,
        store: ClusterTabsStore,
        config: RestoreConfig,
    ) -> Self {
        Self {
            sessions,
            namespaces,
            source,
            store,
            config,
        }
    }

    /// Reads the saved session and reopens its clusters, in tab order, as `Disconnected`
    /// sessions with their remembered namespace selection already applied (the scope must be set
    /// before any feed starts). Nothing connects and nothing touches the network.
    ///
    /// Clusters that no kubeconfig defines any more are not reopened: they are listed in the
    /// plan's `dropped` and removed from the saved session, so the notice appears once.
    ///
    /// # Errors
    ///
    /// The state store's error reading the saved session, or the catalog's error: nothing is
    /// reopened and nothing is dropped (an unreadable catalog proves nothing). The caller logs it
    /// and carries on with an empty window.
    pub async fn prepare(&self) -> OxiResult<RestorePlan> {
        let Some(saved) = self.store.load().await? else {
            return Ok(RestorePlan::default());
        };
        if saved.open.is_empty() {
            return Ok(RestorePlan::default());
        }
        let catalog = self.source.contexts().await?;
        let plan = RestorePlan::resolve(Some(&saved), &catalog);
        if !plan.dropped.is_empty()
            && let Err(error) = self.store.save(&plan.pruned(&saved)).await
        {
            tracing::warn!(%error, "could not remove the vanished clusters from the saved session");
        }
        for context in &plan.clusters {
            self.sessions.open_configured(context);
        }
        for cluster in plan.ids() {
            // A failed read keeps the selection the session opened with.
            if let Err(error) = self.namespaces.restore(cluster).await {
                tracing::warn!(%error, %cluster, "could not restore the namespace selection");
            }
        }
        Ok(plan)
    }

    /// Connects what `connect` asks for: the displayed cluster first, then (for
    /// [`RestoreConnect::All`]) the rest in tab order, at most
    /// [`concurrency`](RestoreConfig::concurrency) at a time, plus one more for clusters whose
    /// credential plugin may prompt (those connect strictly one after another). Each attempt has its own timeout
    /// and its own outcome: a failing or slow cluster never delays or fails another, and a
    /// cluster the user already connected is left alone. With [`RestoreConnect::Active`] and no
    /// displayed cluster nothing connects.
    ///
    /// Dropping the future cancels the attempts that are still running.
    pub async fn connect(&self, plan: &RestorePlan, connect: RestoreConnect) -> RestoreReport {
        let mut queue: Vec<ClusterId> = plan.active.iter().cloned().collect();
        if connect == RestoreConnect::All {
            queue.extend(
                plan.ids()
                    .filter(|c| Some(*c) != plan.active.as_ref())
                    .cloned(),
            );
        }
        // Clusters whose credential plugin may prompt the user go through a lane of their own, one
        // at a time, so dozens of dialogs never open together and a queue of prompts does not
        // hold up the clusters that need none.
        let is_prompting = |cluster: &ClusterId| {
            self.sessions
                .get(cluster)
                .is_some_and(|s| s.exec_interactivity() != ExecInteractivity::Never)
        };
        // The lane of the displayed cluster is polled first, so it starts first.
        let displayed_prompts = queue.first().is_some_and(is_prompting);
        let (prompting, plain): (Vec<_>, Vec<_>) = queue.into_iter().partition(is_prompting);
        let width = self.config.concurrency.max(1);
        let outcomes = if displayed_prompts {
            let (mut outcomes, plain) =
                futures::join!(self.lane(prompting, 1), self.lane(plain, width));
            outcomes.extend(plain);
            outcomes
        } else {
            let (mut outcomes, prompted) =
                futures::join!(self.lane(plain, width), self.lane(prompting, 1));
            outcomes.extend(prompted);
            outcomes
        };
        RestoreReport { outcomes }
    }

    /// Connects `clusters` in order, `width` at a time.
    async fn lane(&self, clusters: Vec<ClusterId>, width: usize) -> Vec<ConnectOutcome> {
        futures::stream::iter(clusters)
            .map(|cluster| self.connect_one(cluster))
            .buffer_unordered(width)
            .collect()
            .await
    }

    /// [`prepare`](Self::prepare) then [`connect`](Self::connect), for callers that have no
    /// placeholders to show in between.
    ///
    /// # Errors
    ///
    /// As [`prepare`](Self::prepare).
    pub async fn restore(
        &self,
        connect: RestoreConnect,
    ) -> OxiResult<(RestorePlan, RestoreReport)> {
        let plan = self.prepare().await?;
        let report = self.connect(&plan, connect).await;
        Ok((plan, report))
    }

    async fn connect_one(&self, cluster: ClusterId) -> ConnectOutcome {
        let skipped = |state: ClusterSessionState| ConnectOutcome {
            cluster: cluster.clone(),
            state,
            attempted: false,
        };
        let Some(session) = self.sessions.get(&cluster) else {
            return skipped(ClusterSessionState::Disconnected);
        };
        if session.phase() != SessionPhase::Disconnected {
            return skipped(session.state().clone());
        }
        let state = match self
            .sessions
            .connect_with_deadline(&cluster, self.config.connect_timeout)
            .await
        {
            Ok(state) => state,
            Err(error) => ClusterSessionState::Error {
                reason: error.to_string(),
            },
        };
        ConnectOutcome {
            cluster,
            state,
            attempted: true,
        }
    }
}

impl std::fmt::Debug for SessionRestorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRestorer")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
