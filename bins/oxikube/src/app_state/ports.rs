//! The ports bundle.

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_describe::DescribePreference;
use oxikube_ports::{
    ClockPort, ClusterConnectorPort, ClusterSourcePort, FsPort, SecretStorePort, StatePort,
};

/// The ports `bins/oxikube` constructs at start-up and hands to everything else as trait
/// objects. Per-cluster ports (resources, discovery, feeds, logs, exec) are not here: the
/// `ClusterSessionManager` gets them per connection from [`ClusterAdapters::connector`].
#[derive(Clone)]
pub struct AppPorts {
    /// Durable local state: key-value, typed tables, audit log (`oxikube_state_sqlite` in the
    /// app, `FakeStatePort` in tests). Never give it a secret.
    pub state: Arc<dyn StatePort>,
    /// The OS keychain. `None` until the keychain adapter is wired (E24 agent tokens); code that
    /// needs it treats `None` as "no secure storage available".
    pub secrets: Option<Arc<dyn SecretStorePort>>,
    /// The cluster side: the kubeconfig catalog and the connector (`oxikube_kube` in the app,
    /// testkit fakes in tests).
    pub clusters: ClusterAdapters,
}

/// The adapters the cluster services run on (E07-S00): what `oxikube_app`'s session manager,
/// catalog, namespace and kubeconfig-source services are built over.
#[derive(Clone)]
pub struct ClusterAdapters {
    /// The kubeconfig contexts (`oxikube_kube::sources` behind `kube_ports::LazyKubeSources`).
    pub source: Arc<dyn ClusterSourcePort>,
    /// Connects a context and returns its ports bundle (`oxikube_kube::KubeConnector`).
    pub connector: Arc<dyn ClusterConnectorPort>,
    /// The wall clock and timers (backoff, debounce, last-used times).
    pub clock: Arc<dyn ClockPort>,
    /// Local files by path (pasted kubeconfigs, owner-only).
    pub fs: Arc<dyn FsPort>,
    /// Where pasted kubeconfigs are stored (`<config dir>/kubeconfigs`, ADR 0015).
    pub kubeconfigs_dir: PathBuf,
    /// Which describe backend the Describe tab uses (`describe.backend`, E07-S06): shared with
    /// every connection's describer, and set from the settings by the mount.
    pub describe: DescribePreference,
}

impl AppPorts {
    /// A bundle with the state port, the cluster adapters and no secret store.
    pub fn new(state: Arc<dyn StatePort>, clusters: ClusterAdapters) -> Self {
        Self {
            state,
            secrets: None,
            clusters,
        }
    }

    /// Adds the secret store.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretStorePort>) -> Self {
        self.secrets = Some(secrets);
        self
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ClusterAdapters {
    /// Where the test bundle "stores" pasted kubeconfigs (the fake file system holds them).
    pub const TEST_KUBECONFIGS_DIR: &'static str = "/oxikube-test/config/kubeconfigs";

    /// The fakes of `ports`: its cluster source, connector, clock and file system.
    pub fn fakes(ports: &oxikube_testkit::TestPorts) -> Self {
        Self {
            source: ports.clusters.clone(),
            connector: ports.connector.clone(),
            clock: ports.clock.clone(),
            fs: ports.fs.clone(),
            kubeconfigs_dir: Self::TEST_KUBECONFIGS_DIR.into(),
            describe: DescribePreference::default(),
        }
    }
}
