//! Behavioural contracts a port's fake and its real adapters must both satisfy, so app tests
//! written against the fake hold for the real thing. Each contract is an async function that
//! drives a port through its normal sequence and panics with a description of the first
//! broken expectation.
//!
//! - [`cloud_discovery`]: `CloudDiscoveryPort` (E18). Run it against
//!   [`FakeCloudDiscoveryPort`](crate::FakeCloudDiscoveryPort) and against each `oxikube_cloud`
//!   adapter fed recorded CLI output.

use std::collections::HashSet;

use oxikube_domain::ErrorKind;
use oxikube_domain::redact::redact;
use oxikube_ports::{
    CloudDiscoveryPort, CloudProvider, CloudScope, CloudToolStatus, DiscoveredCluster,
    KubeconfigEntry,
};

/// What [`cloud_discovery`] checked, so a test can assert it exercised something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CloudContractReport {
    /// Scopes listed.
    pub scopes: usize,
    /// Clusters listed (and turned into kubeconfig entries).
    pub clusters: usize,
}

/// Checks `port` against the `CloudDiscoveryPort` contract:
///
/// 1. `detect` succeeds for the port's own provider, and reports a binary exactly when the CLI
///    is installed and a version when it is ready.
/// 2. A port whose CLI is not installed has no scopes (an `Unsupported` error or an empty list)
///    and the check stops there.
/// 3. `scopes` are all of the port's provider, with distinct keys.
/// 4. `list_clusters(scope)` returns clusters that carry that scope, a name and a location, with
///    keys distinct across all scopes; a scope of another provider is a `Validation` error.
/// 5. `kubeconfig_entry(cluster)` returns an exec-auth entry: a context name, an `https://`
///    server equal to the cluster's endpoint when it had one, its CA when it had one, an exec
///    plugin with a command and a `client.authentication.k8s.io` api version, and nothing
///    secret-looking in its server, arguments or environment. A cluster of another provider is
///    a `Validation` error.
///
/// # Panics
///
/// On the first broken expectation, naming it.
pub async fn cloud_discovery(port: &dyn CloudDiscoveryPort) -> CloudContractReport {
    let provider = port.id();
    let detection = port
        .detect()
        .await
        .unwrap_or_else(|e| panic!("{provider}: detect must not fail: {e}"));
    assert_eq!(
        detection.provider, provider,
        "detect names another provider"
    );
    let installed = detection.status != CloudToolStatus::NotInstalled;
    assert_eq!(
        detection.binary.is_some(),
        installed,
        "{provider}: a binary is reported exactly when the CLI is installed: {detection:?}"
    );
    if detection.status == CloudToolStatus::Ready {
        assert!(
            detection.version.is_some(),
            "{provider}: a ready CLI reports its version"
        );
    }
    if !installed {
        match port.scopes().await {
            Ok(scopes) => assert!(
                scopes.is_empty(),
                "{provider}: no CLI, yet scopes {scopes:?}"
            ),
            Err(e) => assert_eq!(e.kind(), ErrorKind::Unsupported, "{provider}: {e}"),
        }
        return CloudContractReport::default();
    }

    let scopes = port
        .scopes()
        .await
        .unwrap_or_else(|e| panic!("{provider}: scopes failed: {e}"));
    let mut scope_keys = HashSet::new();
    for scope in &scopes {
        assert_eq!(
            scope.provider(),
            provider,
            "scope of another provider: {scope:?}"
        );
        assert!(
            scope_keys.insert(scope.key()),
            "duplicate scope {}",
            scope.key()
        );
    }
    let foreign = foreign_scope(provider);
    let error = port.list_clusters(&foreign).await.err();
    assert_eq!(
        error.map(|e| e.kind()),
        Some(ErrorKind::Validation),
        "{provider}: listing a {} scope must be a Validation error",
        foreign.provider()
    );

    let mut cluster_keys = HashSet::new();
    for scope in &scopes {
        let clusters = port
            .list_clusters(scope)
            .await
            .unwrap_or_else(|e| panic!("{provider}: list_clusters({}) failed: {e}", scope.key()));
        for cluster in clusters {
            assert_eq!(
                &cluster.scope, scope,
                "cluster {} carries another scope",
                cluster.name
            );
            assert!(
                !cluster.name.is_empty(),
                "a cluster in {} has no name",
                scope.key()
            );
            assert!(
                !cluster.location.is_empty(),
                "cluster {} has no location",
                cluster.name
            );
            assert!(
                cluster_keys.insert(cluster.key()),
                "duplicate cluster {}",
                cluster.key()
            );
            let entry = port
                .kubeconfig_entry(&cluster)
                .await
                .unwrap_or_else(|e| panic!("{provider}: entry for {} failed: {e}", cluster.key()));
            check_entry(&cluster, &entry);
        }
    }

    let stranger = DiscoveredCluster::new(foreign, "stranger", "nowhere");
    let error = port.kubeconfig_entry(&stranger).await.err();
    assert_eq!(
        error.map(|e| e.kind()),
        Some(ErrorKind::Validation),
        "{provider}: an entry for another provider's cluster must be a Validation error"
    );
    CloudContractReport {
        scopes: scopes.len(),
        clusters: cluster_keys.len(),
    }
}

fn check_entry(cluster: &DiscoveredCluster, entry: &KubeconfigEntry) {
    let key = cluster.key();
    assert!(
        !entry.context.as_str().is_empty(),
        "{key}: entry without a context name"
    );
    assert!(
        entry.server.starts_with("https://"),
        "{key}: server {} is not https",
        entry.server
    );
    if let Some(endpoint) = &cluster.endpoint {
        assert_eq!(
            &entry.server, endpoint,
            "{key}: server differs from the endpoint"
        );
    }
    if cluster.ca_data.is_some() {
        assert_eq!(
            entry.ca_data, cluster.ca_data,
            "{key}: the cluster's CA was dropped"
        );
    }
    let exec = &entry.exec;
    assert!(
        !exec.command.is_empty(),
        "{key}: exec plugin without a command"
    );
    assert!(
        exec.api_version
            .starts_with("client.authentication.k8s.io/"),
        "{key}: exec api version {}",
        exec.api_version
    );
    let env_values = exec.env.iter().map(|v| v.value.as_str());
    let texts = exec.args.iter().map(String::as_str).chain(env_values);
    for text in texts.chain([entry.server.as_str()]) {
        assert_eq!(
            redact(text),
            text,
            "{key}: entry carries something secret-looking"
        );
    }
    for var in &exec.env {
        let name = var.name.to_ascii_uppercase();
        let secret = ["TOKEN", "SECRET", "PASSWORD", "KEY"]
            .iter()
            .any(|w| name.contains(w));
        assert!(
            !secret,
            "{key}: exec env {} looks like a credential",
            var.name
        );
    }
}

/// A scope of a provider other than `provider`.
fn foreign_scope(provider: CloudProvider) -> CloudScope {
    match provider {
        CloudProvider::Aws => CloudScope::Gcp {
            project: "contract-foreign".to_owned(),
        },
        CloudProvider::Gcp | CloudProvider::Azure => CloudScope::Aws {
            profile: "contract-foreign".to_owned(),
            region: "us-east-1".to_owned(),
        },
    }
}
