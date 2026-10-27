//! [`FakeCloudDiscoveryPort`]: one cloud provider's discovery, from configured state.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    CloudDetection, CloudDiscoveryPort, CloudProvider, CloudScope, CloudToolStatus,
    DiscoveredCluster, ExecEnvVar, ExecInteractiveMode, ExecPlugin, KubeconfigEntry,
};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

/// Queued responses for each [`FakeCloudDiscoveryPort`] method.
#[derive(Debug, Default)]
pub struct CloudScripts {
    /// `detect`.
    pub detect: Script<CloudDetection>,
    /// `scopes`.
    pub scopes: Script<Vec<CloudScope>>,
    /// `list_clusters`.
    pub list_clusters: Script<Vec<DiscoveredCluster>>,
    /// `kubeconfig_entry`.
    pub kubeconfig_entry: Script<KubeconfigEntry>,
}

/// One call made on a [`FakeCloudDiscoveryPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudCall {
    /// `detect()`.
    Detect,
    /// `scopes()`.
    Scopes,
    /// `list_clusters(scope)`.
    ListClusters(CloudScope),
    /// `kubeconfig_entry(cluster)`, by the cluster's [`key`](DiscoveredCluster::key).
    KubeconfigEntry(String),
}

#[derive(Debug)]
struct FakeCloud {
    detection: CloudDetection,
    scopes: Vec<CloudScope>,
    clusters: Vec<DiscoveredCluster>,
    entries: HashMap<String, KubeconfigEntry>,
}

/// Fake `CloudDiscoveryPort` for one provider.
///
/// Fallbacks, which follow the same contract as the real adapters
/// ([`contracts::cloud_discovery`](crate::contracts::cloud_discovery)):
/// `detect` returns the configured detection (`NotInstalled` from [`new`](Self::new), `Ready`
/// from [`ready`](Self::ready)); `scopes` returns the configured scopes; `list_clusters`
/// returns the configured clusters whose scope equals the one asked for; `kubeconfig_entry`
/// returns the entry set with [`with_entry`](Self::with_entry), else builds the provider's
/// usual exec-auth entry (`aws eks get-token`, `gke-gcloud-auth-plugin`, `kubelogin`) with an
/// `https://<name>.<provider>.invalid` server when the cluster has no endpoint. A scope or
/// cluster of another provider is a `Validation` error, as in the adapters.
#[derive(Debug)]
pub struct FakeCloudDiscoveryPort {
    provider: CloudProvider,
    script: CloudScripts,
    calls: CallLog<CloudCall>,
    state: Mutex<FakeCloud>,
}

fake_plumbing!(FakeCloudDiscoveryPort, CloudScripts, CloudCall);

impl FakeCloudDiscoveryPort {
    /// A fake for `provider` whose CLI is missing, with no scopes and no clusters.
    pub fn new(provider: CloudProvider) -> Self {
        Self {
            provider,
            script: CloudScripts::default(),
            calls: CallLog::default(),
            state: Mutex::new(FakeCloud {
                detection: CloudDetection::not_installed(provider),
                scopes: Vec::new(),
                clusters: Vec::new(),
                entries: HashMap::new(),
            }),
        }
    }

    /// A fake for `provider` whose CLI is installed (`/usr/local/bin/<cli>`, version
    /// `0.0.0-fake`) and logged in.
    pub fn ready(provider: CloudProvider) -> Self {
        let detection = CloudDetection {
            provider,
            status: CloudToolStatus::Ready,
            binary: Some(PathBuf::from("/usr/local/bin").join(provider.cli())),
            version: Some("0.0.0-fake".to_owned()),
            hints: Vec::new(),
        };
        Self::new(provider).with_detection(detection)
    }

    /// Sets what `detect` reports.
    #[must_use]
    pub fn with_detection(self, detection: CloudDetection) -> Self {
        self.state.lock().detection = detection;
        self
    }

    /// Sets the scopes `scopes` returns.
    #[must_use]
    pub fn with_scopes(self, scopes: impl IntoIterator<Item = CloudScope>) -> Self {
        self.state.lock().scopes = scopes.into_iter().collect();
        self
    }

    /// Sets the clusters `list_clusters` returns (filtered by scope).
    #[must_use]
    pub fn with_clusters(self, clusters: impl IntoIterator<Item = DiscoveredCluster>) -> Self {
        self.state.lock().clusters = clusters.into_iter().collect();
        self
    }

    /// Sets the entry `kubeconfig_entry` returns for the cluster with `key`.
    #[must_use]
    pub fn with_entry(self, key: impl Into<String>, entry: KubeconfigEntry) -> Self {
        self.state.lock().entries.insert(key.into(), entry);
        self
    }

    fn check_provider(&self, provider: CloudProvider, what: &str) -> OxiResult<()> {
        if provider == self.provider {
            return Ok(());
        }
        Err(OxiError::validation(format!(
            "the {} adapter was given a {what} of {provider}",
            self.provider
        )))
    }
}

/// The exec plugin a real adapter generates for `cluster`.
fn exec_plugin(cluster: &DiscoveredCluster) -> ExecPlugin {
    match &cluster.scope {
        CloudScope::Aws { profile, region } => {
            let args = [
                "eks",
                "get-token",
                "--cluster-name",
                &cluster.name,
                "--region",
                region,
            ];
            ExecPlugin {
                env: vec![ExecEnvVar {
                    name: "AWS_PROFILE".to_owned(),
                    value: profile.clone(),
                }],
                ..ExecPlugin::new(
                    "aws",
                    args.map(str::to_owned).to_vec(),
                    ExecInteractiveMode::IfAvailable,
                )
            }
        }
        CloudScope::Gcp { .. } => ExecPlugin {
            install_hint: Some("gcloud components install gke-gcloud-auth-plugin".to_owned()),
            provide_cluster_info: true,
            ..ExecPlugin::new(
                "gke-gcloud-auth-plugin",
                Vec::new(),
                ExecInteractiveMode::Never,
            )
        },
        CloudScope::Azure { .. } => {
            let args = [
                "get-token",
                "--login",
                "azurecli",
                "--server-id",
                "fake-server-id",
            ];
            ExecPlugin {
                install_hint: Some("az aks install-cli".to_owned()),
                ..ExecPlugin::new(
                    "kubelogin",
                    args.map(str::to_owned).to_vec(),
                    ExecInteractiveMode::IfAvailable,
                )
            }
        }
    }
}

fn default_entry(cluster: &DiscoveredCluster) -> KubeconfigEntry {
    let server = cluster.endpoint.clone().unwrap_or_else(|| {
        format!(
            "https://{}.{}.invalid",
            cluster.name,
            cluster.provider().id()
        )
    });
    KubeconfigEntry {
        context: ContextName::new(cluster.key().replace('/', "-")),
        server,
        ca_data: cluster.ca_data.clone(),
        exec: exec_plugin(cluster),
    }
}

#[async_trait]
impl CloudDiscoveryPort for FakeCloudDiscoveryPort {
    fn id(&self) -> CloudProvider {
        self.provider
    }

    async fn detect(&self) -> OxiResult<CloudDetection> {
        self.calls.record(CloudCall::Detect);
        self.script
            .detect
            .next_or_else(|| Ok(self.state.lock().detection.clone()))
    }

    async fn scopes(&self) -> OxiResult<Vec<CloudScope>> {
        self.calls.record(CloudCall::Scopes);
        self.script
            .scopes
            .next_or_else(|| Ok(self.state.lock().scopes.clone()))
    }

    async fn list_clusters(&self, scope: &CloudScope) -> OxiResult<Vec<DiscoveredCluster>> {
        self.calls.record(CloudCall::ListClusters(scope.clone()));
        self.script.list_clusters.next_or_else(|| {
            self.check_provider(scope.provider(), "scope")?;
            let state = self.state.lock();
            let found = state.clusters.iter().filter(|c| c.scope == *scope);
            Ok(found.cloned().collect())
        })
    }

    async fn kubeconfig_entry(&self, cluster: &DiscoveredCluster) -> OxiResult<KubeconfigEntry> {
        let key = cluster.key();
        self.calls.record(CloudCall::KubeconfigEntry(key.clone()));
        self.script.kubeconfig_entry.next_or_else(|| {
            self.check_provider(cluster.provider(), "cluster")?;
            let configured = self.state.lock().entries.get(&key).cloned();
            Ok(configured.unwrap_or_else(|| default_entry(cluster)))
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use oxikube_domain::ErrorKind;

    use super::*;

    fn aws(profile: &str) -> CloudScope {
        CloudScope::Aws {
            profile: profile.into(),
            region: "eu-west-1".into(),
        }
    }

    #[test]
    fn falls_back_to_configured_state_per_scope() {
        let prod = DiscoveredCluster::new(aws("prod"), "prod", "eu-west-1");
        let dev = DiscoveredCluster::new(aws("dev"), "dev", "eu-west-1");
        let fake = FakeCloudDiscoveryPort::ready(CloudProvider::Aws)
            .with_scopes([aws("prod"), aws("dev")])
            .with_clusters([prod.clone(), dev]);
        assert_eq!(fake.id(), CloudProvider::Aws);
        assert!(block_on(fake.detect()).unwrap().is_ready());
        assert_eq!(block_on(fake.scopes()).unwrap().len(), 2);
        assert_eq!(
            block_on(fake.list_clusters(&aws("prod"))).unwrap(),
            vec![prod.clone()]
        );

        let entry = block_on(fake.kubeconfig_entry(&prod)).unwrap();
        assert_eq!(entry.exec.command, "aws");
        assert_eq!(entry.exec.env[0].value, "prod");
        assert_eq!(entry.server, "https://prod.aws.invalid");
        assert_eq!(entry.context.as_str(), "aws-prod-eu-west-1-eu-west-1-prod");
        assert_eq!(
            fake.recorded_calls(),
            vec![
                CloudCall::Detect,
                CloudCall::Scopes,
                CloudCall::ListClusters(aws("prod")),
                CloudCall::KubeconfigEntry(prod.key()),
            ]
        );
    }

    #[test]
    fn scripts_replace_fallbacks_and_foreign_input_is_rejected() {
        let fake = FakeCloudDiscoveryPort::new(CloudProvider::Gcp);
        assert_eq!(
            block_on(fake.detect()).unwrap().status,
            CloudToolStatus::NotInstalled
        );
        fake.script()
            .scopes
            .push_err(OxiError::auth("run `gcloud auth login`", false));
        assert_eq!(block_on(fake.scopes()).unwrap_err().kind(), ErrorKind::Auth);
        assert!(block_on(fake.scopes()).unwrap().is_empty());
        assert_eq!(
            block_on(fake.list_clusters(&aws("prod")))
                .unwrap_err()
                .kind(),
            ErrorKind::Validation
        );
        let foreign = DiscoveredCluster::new(aws("prod"), "prod", "eu-west-1");
        assert_eq!(
            block_on(fake.kubeconfig_entry(&foreign))
                .unwrap_err()
                .kind(),
            ErrorKind::Validation
        );
    }

    #[test]
    fn configured_entries_win_over_the_generated_one() {
        let scope = CloudScope::Gcp {
            project: "p".into(),
        };
        let cluster = DiscoveredCluster::new(scope, "c", "europe-west1");
        let mut custom = default_entry(&cluster);
        custom.server = "https://203.0.113.7".into();
        let fake = FakeCloudDiscoveryPort::ready(CloudProvider::Gcp)
            .with_entry(cluster.key(), custom.clone());
        assert_eq!(block_on(fake.kubeconfig_entry(&cluster)).unwrap(), custom);
        assert_eq!(
            default_entry(&cluster).exec.command,
            "gke-gcloud-auth-plugin"
        );
    }
}
