//! Session fakes: [`FakeClusterConnectorPort`] (with [`FakeClusterPorts`]) and
//! [`FakeAccessReviewPort`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::oneshot;
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{Capabilities, OxiResult};
use oxikube_ports::{
    AccessReviewPort, ClusterConnection, ClusterConnectorPort, ClusterPorts, ConnectRequest,
    ConnectionGuard, ExecInteractivity, HealthReporter, HealthSignal,
};
use parking_lot::Mutex;

use super::{
    FakeDiscoveryPort, FakeExecPort, FakeLogPort, FakeMetricsPort, FakePortForwardPort,
    FakeResourcePort, FakeTableFeedPort,
};
use crate::script::{CallLog, Script};

// --- AccessReviewPort --------------------------------------------------------------------

/// Queued responses for each [`FakeAccessReviewPort`] method.
#[derive(Debug, Default)]
pub struct AccessScripts {
    /// `capabilities`.
    pub capabilities: Script<Capabilities>,
    /// `rules`.
    pub rules: Script<AccessRules>,
}

/// One call made on a [`FakeAccessReviewPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessCall {
    /// `capabilities(namespace)`.
    Capabilities(Option<String>),
    /// `rules(namespace)`.
    Rules(Option<String>),
}

/// Fake `AccessReviewPort`.
///
/// Fallback: `capabilities` returns the configured set, every flag by default
/// ([`with_capabilities`](Self::with_capabilities), [`set_capabilities`](Self::set_capabilities));
/// `rules` returns the configured rules, cluster admin by default
/// ([`with_rules`](Self::with_rules), [`set_rules`](Self::set_rules)), or the rules configured for
/// that namespace ([`with_namespace_rules`](Self::with_namespace_rules), [`set_namespace_rules`](Self::set_namespace_rules)).
#[derive(Debug)]
pub struct FakeAccessReviewPort {
    script: AccessScripts,
    calls: CallLog<AccessCall>,
    granted: Mutex<Capabilities>,
    rules: Mutex<AccessRules>,
    namespace_rules: Mutex<HashMap<String, AccessRules>>,
}

fake_plumbing!(FakeAccessReviewPort, AccessScripts, AccessCall);

impl Default for FakeAccessReviewPort {
    fn default() -> Self {
        Self {
            script: AccessScripts::default(),
            calls: CallLog::default(),
            granted: Mutex::new(Capabilities::all()),
            rules: Mutex::new(AccessRules::all_access()),
            namespace_rules: Mutex::new(HashMap::new()),
        }
    }
}

impl FakeAccessReviewPort {
    /// A fake that grants every capability.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the granted capabilities.
    #[must_use]
    pub fn with_capabilities(self, granted: Capabilities) -> Self {
        self.set_capabilities(granted);
        self
    }

    /// Replaces the granted capabilities.
    pub fn set_capabilities(&self, granted: Capabilities) {
        *self.granted.lock() = granted;
    }

    /// Sets the rules every `rules` call answers with (unless a namespace has its own).
    #[must_use]
    pub fn with_rules(self, rules: AccessRules) -> Self {
        self.set_rules(rules);
        self
    }

    /// Replaces the rules every `rules` call answers with.
    pub fn set_rules(&self, rules: AccessRules) {
        *self.rules.lock() = rules;
    }

    /// Sets the rules the review of `namespace` answers with.
    #[must_use]
    pub fn with_namespace_rules(self, namespace: &str, rules: AccessRules) -> Self {
        self.set_namespace_rules(namespace, rules);
        self
    }

    /// Replaces the rules the review of `namespace` answers with.
    pub fn set_namespace_rules(&self, namespace: &str, rules: AccessRules) {
        self.namespace_rules
            .lock()
            .insert(namespace.to_owned(), rules);
    }
}

#[async_trait]
impl AccessReviewPort for FakeAccessReviewPort {
    async fn capabilities(&self, namespace: Option<&str>) -> OxiResult<Capabilities> {
        self.calls
            .record(AccessCall::Capabilities(namespace.map(str::to_owned)));
        self.script
            .capabilities
            .next_or_else(|| Ok(*self.granted.lock()))
    }

    async fn rules(&self, namespace: Option<&str>) -> OxiResult<AccessRules> {
        self.calls
            .record(AccessCall::Rules(namespace.map(str::to_owned)));
        self.script.rules.next_or_else(|| {
            let own = namespace.and_then(|ns| self.namespace_rules.lock().get(ns).cloned());
            Ok(own.unwrap_or_else(|| self.rules.lock().clone()))
        })
    }
}

// --- ClusterConnectorPort ----------------------------------------------------------------

/// The typed fakes behind one cluster's [`ClusterPorts`], kept so a test can script them.
#[derive(Clone, Default)]
pub struct FakeClusterPorts {
    /// `ResourcePort`.
    pub resources: Arc<FakeResourcePort>,
    /// `DiscoveryPort`.
    pub discovery: Arc<FakeDiscoveryPort>,
    /// `TableFeedPort`.
    pub tables: Arc<FakeTableFeedPort>,
    /// `LogPort`.
    pub logs: Arc<FakeLogPort>,
    /// `ExecPort`.
    pub exec: Arc<FakeExecPort>,
    /// `PortForwardPort`.
    pub port_forward: Arc<FakePortForwardPort>,
    /// `MetricsPort`.
    pub metrics: Arc<FakeMetricsPort>,
    /// `AccessReviewPort`.
    pub access: Arc<FakeAccessReviewPort>,
}

impl FakeClusterPorts {
    /// The bundle as trait objects.
    pub fn ports(&self) -> ClusterPorts {
        ClusterPorts {
            resources: self.resources.clone(),
            discovery: self.discovery.clone(),
            tables: self.tables.clone(),
            logs: self.logs.clone(),
            exec: self.exec.clone(),
            port_forward: self.port_forward.clone(),
            metrics: self.metrics.clone(),
            access: self.access.clone(),
        }
    }
}

/// Queued responses for each [`FakeClusterConnectorPort`] method.
#[derive(Debug, Default)]
pub struct ConnectorScripts {
    /// `connect`: `Ok(())` connects with the cluster's [`FakeClusterPorts`], an error fails.
    pub connect: Script<()>,
}

/// One call made on a [`FakeClusterConnectorPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorCall {
    /// `connect(request)`, without the health reporter.
    Connect {
        /// `request.cluster`.
        cluster: ClusterId,
        /// `request.context`.
        context: ContextName,
        /// `request.exec_interactivity`.
        exec_interactivity: ExecInteractivity,
    },
}

#[derive(Default)]
struct ConnectorState {
    ports: HashMap<ClusterId, FakeClusterPorts>,
    scripts: HashMap<ClusterId, Arc<Script<()>>>,
    reporters: HashMap<ClusterId, Arc<dyn HealthReporter>>,
    holding: bool,
    held: Vec<oneshot::Sender<()>>,
    cancelled: usize,
    live: HashMap<ClusterId, Arc<()>>,
}

/// Fake `ClusterConnectorPort`.
///
/// Responses: a connect to a cluster takes the next response from that cluster's
/// [`connect_script_for`](Self::connect_script_for), then from the shared
/// [`ConnectorScripts::connect`]. Fallback: `connect` succeeds with the cluster's
/// [`FakeClusterPorts`] (created on first use and reused by later connects; script them
/// through [`ports_for`](Self::ports_for)).
///
/// Test controls:
///
/// * [`hold`](Self::hold) / [`release`](Self::release): while holding, `connect` waits
///   before taking its scripted response, so a test can observe `Connecting` and cancel
///   it. [`held`](Self::held) counts the waiting calls, [`cancelled`](Self::cancelled) the
///   ones dropped while waiting.
/// * [`report`](Self::report) sends a [`HealthSignal`] through the reporter of the
///   cluster's latest connect, as the adapter's liveness loop would.
/// * [`live_connections`](Self::live_connections) counts connections whose
///   [`ConnectionGuard`] is still alive.
#[derive(Default)]
pub struct FakeClusterConnectorPort {
    script: ConnectorScripts,
    calls: CallLog<ConnectorCall>,
    state: Mutex<ConnectorState>,
}

fake_plumbing!(FakeClusterConnectorPort, ConnectorScripts, ConnectorCall);

impl std::fmt::Debug for FakeClusterConnectorPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        f.debug_struct("FakeClusterConnectorPort")
            .field("clusters", &state.ports.len())
            .field("holding", &state.holding)
            .field("held", &state.held.len())
            .finish_non_exhaustive()
    }
}

impl FakeClusterConnectorPort {
    /// A connector whose connects all succeed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The fakes `cluster` connects with (created on first use).
    pub fn ports_for(&self, cluster: &ClusterId) -> FakeClusterPorts {
        self.state
            .lock()
            .ports
            .entry(cluster.clone())
            .or_default()
            .clone()
    }

    /// The connect responses for `cluster` only (created on first use). They are taken
    /// before the shared [`ConnectorScripts::connect`], when the call reads its response:
    /// after [`hold`](Self::hold) lets it go, so a held connect to `cluster` gets what is
    /// queued here at release, whatever other clusters' connects consume meanwhile.
    pub fn connect_script_for(&self, cluster: &ClusterId) -> Arc<Script<()>> {
        self.state
            .lock()
            .scripts
            .entry(cluster.clone())
            .or_default()
            .clone()
    }

    /// Makes every later `connect` wait until [`release`](Self::release).
    pub fn hold(&self) {
        self.state.lock().holding = true;
    }

    /// Stops holding and lets every waiting `connect` continue.
    pub fn release(&self) {
        let held = {
            let mut state = self.state.lock();
            state.holding = false;
            std::mem::take(&mut state.held)
        };
        for waiter in held {
            let _ = waiter.send(());
        }
    }

    /// Number of `connect` calls waiting on [`hold`](Self::hold).
    pub fn held(&self) -> usize {
        let mut state = self.state.lock();
        state.held.retain(|w| !w.is_canceled());
        state.held.len()
    }

    /// Number of `connect` futures dropped while they were held.
    pub fn cancelled(&self) -> usize {
        self.state.lock().cancelled
    }

    /// The health reporter of `cluster`'s latest connect.
    pub fn reporter(&self, cluster: &ClusterId) -> Option<Arc<dyn HealthReporter>> {
        self.state.lock().reporters.get(cluster).cloned()
    }

    /// Sends `signal` through the health reporter of `cluster`'s latest connect. Returns
    /// whether there was one.
    pub fn report(&self, cluster: &ClusterId, signal: HealthSignal) -> bool {
        self.reporter(cluster).map(|r| r.report(signal)).is_some()
    }

    /// Number of connections to `cluster` whose guard has not been dropped.
    pub fn live_connections(&self, cluster: &ClusterId) -> usize {
        self.state
            .lock()
            .live
            .get(cluster)
            .map_or(0, |token| Arc::strong_count(token) - 1)
    }

    async fn wait_if_held(&self) {
        let waiter = {
            let mut state = self.state.lock();
            if !state.holding {
                return;
            }
            let (tx, rx) = oneshot::channel();
            state.held.push(tx);
            rx
        };
        // Counts a cancellation if this future is dropped before the release arrives.
        struct Cancelled<'a>(&'a FakeClusterConnectorPort, bool);
        impl Drop for Cancelled<'_> {
            fn drop(&mut self) {
                if self.1 {
                    self.0.state.lock().cancelled += 1;
                }
            }
        }
        let mut cancelled = Cancelled(self, true);
        let _ = waiter.await;
        cancelled.1 = false;
    }
}

#[async_trait]
impl ClusterConnectorPort for FakeClusterConnectorPort {
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection> {
        self.calls.record(ConnectorCall::Connect {
            cluster: request.cluster.clone(),
            context: request.context.clone(),
            exec_interactivity: request.exec_interactivity,
        });
        self.wait_if_held().await;
        let own = self.state.lock().scripts.get(&request.cluster).cloned();
        own.and_then(|script| script.pop())
            .unwrap_or_else(|| self.script.connect.next_or_else(|| Ok(())))?;
        let mut state = self.state.lock();
        state
            .reporters
            .insert(request.cluster.clone(), request.health.clone());
        let ports = state
            .ports
            .entry(request.cluster.clone())
            .or_default()
            .ports();
        let token = state.live.entry(request.cluster).or_default().clone();
        Ok(ClusterConnection {
            ports,
            guard: ConnectionGuard::new(token),
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use futures::executor::block_on;
    use oxikube_domain::OxiError;

    use super::*;

    struct NoopReporter;
    impl HealthReporter for NoopReporter {
        fn report(&self, _: HealthSignal) {}
    }

    fn request(name: &str) -> ConnectRequest {
        let context = ContextName::new(name);
        ConnectRequest {
            cluster: ClusterId::new("/kube/config", &context),
            context,
            exec_interactivity: ExecInteractivity::Never,
            health: Arc::new(NoopReporter),
        }
    }

    #[test]
    fn connect_scripts_then_falls_back_to_success() {
        let fake = FakeClusterConnectorPort::new();
        fake.script()
            .connect
            .push_err(OxiError::auth("login needed", false));
        let req = request("a");
        let err = block_on(fake.connect(req.clone())).unwrap_err();
        assert_eq!(err.kind(), oxikube_domain::ErrorKind::Auth);
        let conn = block_on(fake.connect(req.clone())).unwrap();
        assert_eq!(fake.live_connections(&req.cluster), 1);
        drop(conn);
        assert_eq!(fake.live_connections(&req.cluster), 0);
        assert_eq!(fake.recorded_calls().len(), 2);
    }

    #[test]
    fn per_cluster_scripts_win_over_the_shared_one_and_apply_at_release() {
        let fake = FakeClusterConnectorPort::new();
        let (a, b) = (request("a"), request("b"));
        fake.script().connect.push_err(OxiError::internal("shared"));
        fake.hold();
        let mut held = fake.connect(a.clone()).boxed();
        assert!((&mut held).now_or_never().is_none());
        // Scripted after the call started: still read, because the response is taken
        // only once the hold is released.
        fake.connect_script_for(&a.cluster)
            .push_err(OxiError::auth("login needed", false));
        fake.release();
        let err = block_on(held).unwrap_err();
        assert_eq!(err.kind(), oxikube_domain::ErrorKind::Auth);
        // `b` has no script of its own and takes the shared response; `a` then falls back.
        let err = block_on(fake.connect(b)).unwrap_err();
        assert_eq!(err.kind(), oxikube_domain::ErrorKind::Internal);
        assert!(block_on(fake.connect(a)).is_ok());
    }

    #[test]
    fn held_connects_wait_and_count_cancellations() {
        let fake = FakeClusterConnectorPort::new();
        fake.hold();
        let mut pending = fake.connect(request("a")).boxed();
        assert!((&mut pending).now_or_never().is_none());
        assert_eq!(fake.held(), 1);
        drop(pending);
        assert_eq!(fake.cancelled(), 1);
        assert_eq!(fake.held(), 0);

        let mut pending = fake.connect(request("b")).boxed();
        assert!((&mut pending).now_or_never().is_none());
        fake.release();
        assert!(block_on(pending).is_ok());
        assert_eq!(fake.cancelled(), 1);
    }

    #[test]
    fn access_fake_returns_configured_capabilities() {
        let fake = FakeAccessReviewPort::new().with_capabilities(Capabilities::LOGS);
        assert_eq!(
            block_on(fake.capabilities(Some("dev"))).unwrap(),
            Capabilities::LOGS
        );
        assert_eq!(
            fake.recorded_calls(),
            vec![AccessCall::Capabilities(Some("dev".into()))]
        );
    }
}
