//! [`ClusterSessionManager`]: the public face of the session service.

use std::sync::Arc;

use indexmap::IndexMap;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, NamespaceSelection, SessionPhase};
use oxikube_domain::{ClusterColour, OxiError, OxiResult};
use oxikube_ports::{
    ClockPort, ClusterConnectorPort, ClusterContext, ClusterPrefsTable, ClusterSourcePort,
    ExecInteractivity, HealthSignal,
};
use parking_lot::{Mutex, RwLock};

use super::config::{SessionManagerConfig, SessionOptions};
use super::entry::{Entry, Released};
use super::model::ClusterSession;
use super::updates::{SessionChange, SessionUpdates, UpdateSender};

/// The one authority on which clusters are open, in what state, with which ports,
/// namespace selection, colour and read-only flag. See the [module docs](super).
///
/// Cheap to clone; clones share the sessions.
#[derive(Clone)]
pub struct ClusterSessionManager {
    pub(super) shared: Arc<Shared>,
}

pub(super) struct Shared {
    pub(super) connector: Arc<dyn ClusterConnectorPort>,
    source: Arc<dyn ClusterSourcePort>,
    pub(super) clock: Arc<dyn ClockPort>,
    pub(super) config: SessionManagerConfig,
    /// Open order is kept for `sessions()`. Locked only to find or add an entry, never
    /// while an entry is locked by the same code path for longer than a lookup.
    pub(super) sessions: RwLock<IndexMap<ClusterId, Arc<Mutex<Entry>>>>,
    /// The per-cluster settings pushed by the binary (E06-S08); a lookup index by cluster id.
    pub(super) prefs: RwLock<Arc<ClusterPrefsTable>>,
    pub(super) updates: UpdateSender,
}

impl std::fmt::Debug for ClusterSessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterSessionManager")
            .field("sessions", &self.shared.sessions.read().len())
            .finish_non_exhaustive()
    }
}

impl ClusterSessionManager {
    /// A manager with the default configuration.
    ///
    /// `source` resolves ids that were not [`open`](Self::open)ed yet; `clock` times the
    /// retry backoff.
    pub fn new(
        connector: Arc<dyn ClusterConnectorPort>,
        source: Arc<dyn ClusterSourcePort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self::with_config(connector, source, clock, SessionManagerConfig::default())
    }

    /// A manager with `config`.
    pub fn with_config(
        connector: Arc<dyn ClusterConnectorPort>,
        source: Arc<dyn ClusterSourcePort>,
        clock: Arc<dyn ClockPort>,
        config: SessionManagerConfig,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                connector,
                source,
                clock,
                config,
                sessions: RwLock::new(IndexMap::new()),
                prefs: RwLock::new(Arc::new(ClusterPrefsTable::default())),
                updates: UpdateSender::new(config.update_capacity),
            }),
        }
    }

    /// Subscribes to every session's updates from now on.
    pub fn subscribe(&self) -> SessionUpdates {
        self.shared.updates.subscribe()
    }

    /// Opens a `Disconnected` session for `context` with `options`, or returns the
    /// existing session unchanged when it is already open.
    pub fn open(&self, context: &ClusterContext, options: SessionOptions) -> ClusterSession {
        self.shared.open(context, options).lock().snapshot()
    }

    /// Disconnects (if needed) and forgets the session. Returns whether it was open.
    pub fn close(&self, cluster: &ClusterId) -> bool {
        let Some(entry) = self.shared.sessions.write().shift_remove(cluster) else {
            return false;
        };
        let released = {
            let mut e = entry.lock();
            let released = e.disconnect(&self.shared.updates);
            // Late results and reports must not land in a closed session.
            e.generation += 1;
            self.shared.updates.send(cluster, SessionChange::Closed);
            released
        };
        drop(released);
        true
    }

    /// A snapshot of one session.
    pub fn get(&self, cluster: &ClusterId) -> Option<ClusterSession> {
        self.shared.entry(cluster).map(|e| e.lock().snapshot())
    }

    /// Snapshots of every open session, in open order.
    pub fn sessions(&self) -> Vec<ClusterSession> {
        let entries: Vec<_> = self.shared.sessions.read().values().cloned().collect();
        entries.iter().map(|e| e.lock().snapshot()).collect()
    }

    /// Connects `cluster` and returns the state the attempt ended in: `Ready`,
    /// `AuthRequired` (credentials needed; call again to retry), `Error`, or
    /// `Disconnected` when [`disconnect`](Self::disconnect) cancelled it.
    ///
    /// A session that is not open yet is opened from the cluster source's catalog, with the
    /// options its settings give ([`open_configured`](Self::open_configured)). When the session
    /// is already `Connecting`, `Ready` or `Degraded`, nothing starts and the current state is
    /// returned; use [`reconnect`](Self::reconnect) to force a new connection.
    ///
    /// Run it off the UI thread (`oxikube_runtime::spawn_kube`). Dropping the future
    /// cancels the attempt and returns the session to `Disconnected`.
    ///
    /// # Errors
    ///
    /// `NotFound` when `cluster` is neither open nor in the catalog; the cluster
    /// source's error when the catalog cannot be read. Connection failures are states,
    /// not errors.
    pub async fn connect(&self, cluster: &ClusterId) -> OxiResult<ClusterSessionState> {
        let entry = match self.shared.entry(cluster) {
            Some(entry) => entry,
            None => self.open_from_catalog(cluster).await?,
        };
        Ok(self.shared.run_connect(entry).await)
    }

    /// Drops the connection (tearing down its feeds and health loop) and moves the
    /// session to `Disconnected`. Cancels an in-flight [`connect`](Self::connect). A
    /// no-op on a session that is already `Disconnected`.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open.
    pub fn disconnect(&self, cluster: &ClusterId) -> OxiResult<()> {
        let entry = self.shared.entry(cluster).ok_or_else(|| unknown(cluster))?;
        let released = entry.lock().disconnect(&self.shared.updates);
        drop(released);
        Ok(())
    }

    /// Disconnects when connected or connecting, then [`connect`](Self::connect)s.
    ///
    /// # Errors
    ///
    /// As [`connect`](Self::connect).
    pub async fn reconnect(&self, cluster: &ClusterId) -> OxiResult<ClusterSessionState> {
        if let Some(entry) = self.shared.entry(cluster) {
            let released = {
                let mut e = entry.lock();
                match e.phase() {
                    SessionPhase::Connecting | SessionPhase::Ready | SessionPhase::Degraded => {
                        e.disconnect(&self.shared.updates)
                    }
                    _ => Released(None),
                }
            };
            drop(released);
        }
        self.connect(cluster).await
    }

    /// Reports the health of a connected session from outside the connector, for
    /// example a feed that keeps failing (`Unhealthy`) and then recovers (`Healthy`).
    /// Ignored unless the session is `Ready` or `Degraded`. Returns whether it applied.
    pub fn report_health(&self, cluster: &ClusterId, signal: HealthSignal) -> bool {
        self.shared.on_health(cluster, None, signal)
    }

    /// Sets the namespace selection. Returns whether it changed.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open.
    pub fn set_namespace_selection(
        &self,
        cluster: &ClusterId,
        selection: NamespaceSelection,
    ) -> OxiResult<bool> {
        self.shared.update(cluster, |e| {
            (e.namespace_selection != selection).then(|| {
                e.namespace_selection = selection.clone();
                SessionChange::NamespaceChanged(selection)
            })
        })
    }

    /// Sets the read-only flag. Returns whether it changed.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open.
    pub fn set_read_only(&self, cluster: &ClusterId, read_only: bool) -> OxiResult<bool> {
        self.shared.update(cluster, |e| {
            (e.read_only != read_only).then(|| {
                e.read_only = read_only;
                SessionChange::ReadOnlyChanged(read_only)
            })
        })
    }

    /// Sets the colour. Returns whether it changed.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open.
    pub fn set_colour(
        &self,
        cluster: &ClusterId,
        colour: Option<ClusterColour>,
    ) -> OxiResult<bool> {
        self.shared.update(cluster, |e| {
            (e.colour != colour).then(|| {
                e.colour = colour;
                SessionChange::ColourChanged(colour)
            })
        })
    }

    /// Sets the exec credential plugin policy for the next connect (no update is sent).
    /// Returns whether it changed.
    ///
    /// # Errors
    ///
    /// `NotFound` when the session is not open.
    pub fn set_exec_interactivity(
        &self,
        cluster: &ClusterId,
        policy: ExecInteractivity,
    ) -> OxiResult<bool> {
        let entry = self.shared.entry(cluster).ok_or_else(|| unknown(cluster))?;
        let mut e = entry.lock();
        let changed = e.exec_interactivity != policy;
        e.exec_interactivity = policy;
        Ok(changed)
    }

    async fn open_from_catalog(&self, cluster: &ClusterId) -> OxiResult<Arc<Mutex<Entry>>> {
        let contexts = self.shared.source.contexts().await?;
        let context = contexts
            .iter()
            .find(|c| &c.cluster == cluster)
            .ok_or_else(|| unknown(cluster))?;
        Ok(self.shared.open_configured(context))
    }
}

impl Shared {
    pub(super) fn entry(&self, cluster: &ClusterId) -> Option<Arc<Mutex<Entry>>> {
        self.sessions.read().get(cluster).cloned()
    }

    fn open(&self, context: &ClusterContext, options: SessionOptions) -> Arc<Mutex<Entry>> {
        self.open_with(context, |_| options)
    }

    /// Opens `context` with the options its settings give. They are made while the session list
    /// is locked, so a concurrent `set_prefs_table` either sees the new session (and applies to
    /// it) or runs first (and the session starts from it).
    pub(super) fn open_configured(&self, context: &ClusterContext) -> Arc<Mutex<Entry>> {
        self.open_with(context, |table| {
            SessionOptions::from_prefs(
                table.get(&context.cluster),
                context.default_namespace.as_deref(),
            )
        })
    }

    fn open_with(
        &self,
        context: &ClusterContext,
        options: impl FnOnce(&ClusterPrefsTable) -> SessionOptions,
    ) -> Arc<Mutex<Entry>> {
        let mut sessions = self.sessions.write();
        if let Some(entry) = sessions.get(&context.cluster) {
            return entry.clone();
        }
        let options = options(&self.prefs.read());
        let entry = Entry::new(context.cluster.clone(), context.context.clone(), options);
        let entry = Arc::new(Mutex::new(entry));
        sessions.insert(context.cluster.clone(), entry.clone());
        self.updates.send(&context.cluster, SessionChange::Opened);
        entry
    }

    fn update(
        &self,
        cluster: &ClusterId,
        change: impl FnOnce(&mut Entry) -> Option<SessionChange>,
    ) -> OxiResult<bool> {
        let entry = self.entry(cluster).ok_or_else(|| unknown(cluster))?;
        let mut e = entry.lock();
        Ok(match change(&mut e) {
            Some(change) => {
                self.updates.send(cluster, change);
                true
            }
            None => false,
        })
    }

    /// Applies a health signal to a connected session. `generation` is the connection
    /// the report is about (`None`: whatever is connected now).
    pub(super) fn on_health(
        &self,
        cluster: &ClusterId,
        generation: Option<u64>,
        signal: HealthSignal,
    ) -> bool {
        let Some(entry) = self.entry(cluster) else {
            return false;
        };
        let released = {
            let mut e = entry.lock();
            let current = generation.is_none_or(|g| g == e.generation);
            if !current || !e.phase().is_connected() {
                tracing::trace!(%cluster, ?signal, "health report ignored");
                return false;
            }
            let Ok(released) = e.apply(signal.to_session_event(), &self.updates) else {
                return false;
            };
            released
        };
        drop(released);
        true
    }
}

/// The error for an unknown session id.
fn unknown(cluster: &ClusterId) -> OxiError {
    OxiError::not_found(format!("no cluster session or catalog entry {cluster}"))
}
