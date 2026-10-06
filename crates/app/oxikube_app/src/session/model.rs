//! [`ClusterSession`]: a point-in-time view of one session.

use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ContextName, Scope};
use oxikube_domain::session::{ClusterSessionState, NamespaceSelection, SessionPhase, WatchScope};
use oxikube_domain::{Capabilities, Capability, ClusterColour};
use oxikube_ports::{
    AccessReviewPort, ClusterPorts, ClusterPrefs, DiscoveryPort, ExecInteractivity, ExecPort,
    LogPort, MetricsPort, PortForwardPort, ResourceReader, ResourceWriter, TableFeedPort,
};

/// One cluster session as the manager saw it when the snapshot was taken.
///
/// Cheap to clone. It does not update itself: subscribe to
/// [`updates`](super::ClusterSessionManager::subscribe) and re-read with
/// [`get`](super::ClusterSessionManager::get). The ports are present only while the
/// session is connected (`Ready` or `Degraded`); a snapshot taken then keeps them alive
/// until it is dropped, so do not hold one longer than the work that needs it.
///
/// The resource port is handed out as a [`ResourceReader`] only: writes go through
/// `MutationGuard` (E06-S02), never through a session snapshot (non-negotiable 3).
#[derive(Debug, Clone)]
pub struct ClusterSession {
    pub(super) id: ClusterId,
    pub(super) context: ContextName,
    pub(super) state: ClusterSessionState,
    pub(super) capabilities: Capabilities,
    pub(super) namespace_selection: NamespaceSelection,
    pub(super) read_only: bool,
    pub(super) colour: Option<ClusterColour>,
    pub(super) exec_interactivity: ExecInteractivity,
    pub(super) display_name: Option<String>,
    pub(super) server: Option<String>,
    pub(super) prefs: Arc<ClusterPrefs>,
    pub(super) ports: Option<ClusterPorts>,
}

impl ClusterSession {
    /// The catalog entry.
    pub fn id(&self) -> &ClusterId {
        &self.id
    }

    /// The kubeconfig context name.
    pub fn context(&self) -> &ContextName {
        &self.context
    }

    /// The name the user gave the cluster (`display_name` in its settings), if any.
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// The API server URL of the catalog entry the session was opened from, if it names one.
    /// Plain text from the kubeconfig (never a credential); redact it before it is shown.
    pub fn server(&self) -> Option<&str> {
        self.server.as_deref()
    }

    /// What to call the cluster in the UI: its display name, else the context name.
    pub fn title(&self) -> &str {
        self.display_name
            .as_deref()
            .unwrap_or_else(|| self.context.as_str())
    }

    /// The cluster's resolved settings as last pushed (default namespace, terminal directory,
    /// node shell, Prometheus override, accessible namespaces, ...). `read_only`, the colour,
    /// the display name and the exec policy are also available as the live fields above,
    /// which follow these settings and can be moved by the session's own setters.
    pub fn prefs(&self) -> &ClusterPrefs {
        &self.prefs
    }

    /// The connection state.
    pub fn state(&self) -> &ClusterSessionState {
        &self.state
    }

    /// The connection phase.
    pub fn phase(&self) -> SessionPhase {
        self.state.phase()
    }

    /// Whether the session is `Ready` or `Degraded`.
    pub fn is_connected(&self) -> bool {
        self.phase().is_connected()
    }

    /// What the user may do, probed on connect; empty while not connected.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Whether the session has `capability`.
    pub fn can(&self, capability: Capability) -> bool {
        self.capabilities.contains(capability.flag())
    }

    /// Which namespaces the session watches.
    pub fn namespace_selection(&self) -> &NamespaceSelection {
        &self.namespace_selection
    }

    /// The watch shape for a kind of `kind_scope` under the current selection.
    pub fn watch_scope(&self, kind_scope: Scope) -> WatchScope {
        WatchScope::derive(&self.namespace_selection, kind_scope)
    }

    /// Whether mutations are blocked.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// The accent colour.
    pub fn colour(&self) -> Option<ClusterColour> {
        self.colour
    }

    /// The exec credential plugin policy used on the next connect.
    pub fn exec_interactivity(&self) -> ExecInteractivity {
        self.exec_interactivity
    }

    /// Reads cluster objects (`None` while not connected).
    pub fn resources(&self) -> Option<Arc<dyn ResourceReader>> {
        self.ports
            .as_ref()
            .map(|p| p.resources.clone() as Arc<dyn ResourceReader>)
    }

    /// The writer half of the resource port (`None` while not connected).
    ///
    /// Crate-private on purpose: only [`MutationGuard`](crate::guard::MutationGuard) calls
    /// it, and it hands the writer to a handler inside a
    /// [`Mutation`](crate::guard::Mutation) only after the read-only check, the
    /// confirmation and the audit precondition passed (non-negotiable 3).
    pub(crate) fn writer(&self) -> Option<Arc<dyn ResourceWriter>> {
        self.ports
            .as_ref()
            .map(|p| p.resources.clone() as Arc<dyn ResourceWriter>)
    }

    /// API discovery.
    pub fn discovery(&self) -> Option<Arc<dyn DiscoveryPort>> {
        self.ports.as_ref().map(|p| p.discovery.clone())
    }

    /// Server-side Table feeds.
    pub fn tables(&self) -> Option<Arc<dyn TableFeedPort>> {
        self.ports.as_ref().map(|p| p.tables.clone())
    }

    /// Container logs.
    pub fn logs(&self) -> Option<Arc<dyn LogPort>> {
        self.ports.as_ref().map(|p| p.logs.clone())
    }

    /// Exec and attach.
    pub fn exec(&self) -> Option<Arc<dyn ExecPort>> {
        self.ports.as_ref().map(|p| p.exec.clone())
    }

    /// Port-forwarding.
    pub fn port_forward(&self) -> Option<Arc<dyn PortForwardPort>> {
        self.ports.as_ref().map(|p| p.port_forward.clone())
    }

    /// `metrics.k8s.io`.
    pub fn metrics(&self) -> Option<Arc<dyn MetricsPort>> {
        self.ports.as_ref().map(|p| p.metrics.clone())
    }

    /// The access review port, to re-probe capabilities for a namespace.
    pub fn access(&self) -> Option<Arc<dyn AccessReviewPort>> {
        self.ports.as_ref().map(|p| p.access.clone())
    }
}
