//! Kind integration tests for `KubeDiscovery` (E03-S06). Need `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skip cleanly otherwise.
#![cfg(feature = "integration")]

use std::process::Command;
use std::time::{Duration, Instant};

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{DeleteParams, PostParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_kube::{CrdWatchConfig, DiscoveryConfig, KubeDiscovery, RegistryDiff};
use oxikube_ports::DiscoveryPort;
use oxikube_testkit::integration::{ensure_kind_context, test_context};
use tokio::sync::broadcast::Receiver;
use tokio::time::timeout;

/// Upper bound for the cluster to reflect a CRD change in discovery.
const DEADLINE: Duration = Duration::from_secs(30);

async fn client_for(context: &str) -> Client {
    ensure_kind_context(context).expect("kind context");
    let options = KubeConfigOptions {
        context: Some(context.to_owned()),
        ..Default::default()
    };
    let kubeconfig = Kubeconfig::read().expect("read kubeconfig");
    let config = Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .expect("kubeconfig for context");
    Client::try_from(config).expect("client")
}

/// A CRD created by a test; deleted (best effort, no wait) when dropped, also on panic.
struct TestCrd {
    context: String,
    name: String,
    gvk: Gvk,
}

impl TestCrd {
    /// Creates `gizmos.rt-<rand>.test.oxikube.dev` (kind `Gizmo`, version `v1`).
    async fn create(client: &Client, context: &str) -> Self {
        let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let group = format!("rt-{suffix}.test.oxikube.dev");
        let name = format!("gizmos.{group}");
        let crd: CustomResourceDefinition = serde_json::from_value(serde_json::json!({
            "apiVersion": "apiextensions.k8s.io/v1",
            "kind": "CustomResourceDefinition",
            "metadata": {"name": name},
            "spec": {
                "group": group,
                "scope": "Namespaced",
                "names": {
                    "plural": "gizmos", "singular": "gizmo", "kind": "Gizmo",
                    "listKind": "GizmoList", "shortNames": ["gz"],
                },
                "versions": [{
                    "name": "v1", "served": true, "storage": true,
                    "schema": {"openAPIV3Schema": {"type": "object", "x-kubernetes-preserve-unknown-fields": true}},
                }],
            },
        }))
        .expect("crd json");
        // Registered before the create so a failed or interrupted create still cleans up.
        let guard = Self {
            context: context.to_owned(),
            name,
            gvk: Gvk::new(group, "v1", "Gizmo"),
        };
        Api::<CustomResourceDefinition>::all(client.clone())
            .create(&PostParams::default(), &crd)
            .await
            .expect("create CRD");
        guard
    }

    async fn delete(&self, client: &Client) {
        Api::<CustomResourceDefinition>::all(client.clone())
            .delete(&self.name, &DeleteParams::default())
            .await
            .expect("delete CRD");
    }
}

impl Drop for TestCrd {
    fn drop(&mut self) {
        let _ = Command::new("kubectl")
            .args(["--context", &self.context, "delete", "crd", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}

/// Waits for a published diff for which `wanted` holds, within [`DEADLINE`].
async fn next_diff(
    changes: &mut Receiver<std::sync::Arc<RegistryDiff>>,
    what: &str,
    wanted: impl Fn(&RegistryDiff) -> bool,
) -> std::sync::Arc<RegistryDiff> {
    let started = Instant::now();
    loop {
        let remaining = DEADLINE.saturating_sub(started.elapsed());
        match timeout(remaining, changes.recv()).await {
            Ok(Ok(diff)) if wanted(&diff) => return diff,
            Ok(Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(err)) => panic!("change channel closed while waiting for {what}: {err}"),
            Err(_) => panic!("{what} not seen within {DEADLINE:?}"),
        }
    }
}

#[tokio::test]
async fn registry_lists_builtin_kinds_and_the_fixture_crd() {
    let Some(ctx) = test_context() else { return };
    let discovery = KubeDiscovery::new(client_for(&ctx).await);

    let started = Instant::now();
    let kinds = discovery.discover().await.expect("discover");
    let elapsed = started.elapsed();
    eprintln!("aggregated discovery: {} kinds in {elapsed:?}", kinds.len());

    let get = |group: &str, version: &str, kind: &str| {
        kinds
            .iter()
            .find(|k| k.gvk == Gvk::new(group, version, kind))
            .unwrap_or_else(|| panic!("{group}/{version} {kind} not discovered"))
    };
    let pod = get("", "v1", "Pod");
    assert!(pod.namespaced && pod.preferred);
    assert_eq!(pod.short_names, ["po"]);
    assert!(pod.categories.iter().any(|c| c == "all"));
    assert!(pod.supports(Verb::List) && pod.is_watchable());

    let deployment = get("apps", "v1", "Deployment");
    assert_eq!(deployment.short_names, ["deploy"]);
    assert_eq!(deployment.plural, "deployments");

    assert!(!get("", "v1", "Namespace").namespaced);

    let widget = get("test.oxikube.dev", "v1", "Widget");
    assert_eq!(widget.plural, "widgets");
    assert!(widget.namespaced && widget.preferred);

    assert!(
        kinds.iter().all(|k| !k.plural.contains('/')),
        "subresources are not listed"
    );
}

#[tokio::test]
async fn aggregated_and_legacy_discovery_agree_on_a_real_cluster() {
    let Some(ctx) = test_context() else { return };
    let client = client_for(&ctx).await;
    let aggregated = KubeDiscovery::new(client.clone());
    let legacy = KubeDiscovery::with_config(
        client,
        DiscoveryConfig {
            aggregated: false,
            ..DiscoveryConfig::default()
        },
    );

    let started = Instant::now();
    let from_aggregated = aggregated.discover().await.expect("aggregated");
    let aggregated_time = started.elapsed();
    let started = Instant::now();
    let from_legacy = legacy.discover().await.expect("legacy");
    let legacy_time = started.elapsed();
    eprintln!(
        "discovery on kind: aggregated {aggregated_time:?}, legacy fallback {legacy_time:?} ({} kinds)",
        from_aggregated.len()
    );

    assert_eq!(from_aggregated, from_legacy);
}

#[tokio::test]
async fn a_crd_created_at_runtime_appears_and_removal_is_reported() {
    let Some(ctx) = test_context() else { return };
    let client = client_for(&ctx).await;
    let discovery = KubeDiscovery::new(client.clone());
    discovery.discover().await.expect("discover");
    let mut changes = discovery.subscribe();
    let _watch = discovery.watch_crds(CrdWatchConfig {
        debounce: Duration::from_millis(100),
        ..CrdWatchConfig::default()
    });

    let crd = TestCrd::create(&client, &ctx).await;
    let diff = next_diff(&mut changes, "the new CRD's kind", |d| {
        d.added.iter().any(|k| k.gvk == crd.gvk)
    })
    .await;
    let gizmo = diff.added.iter().find(|k| k.gvk == crd.gvk).expect("added");
    assert_eq!(
        (gizmo.plural.as_str(), gizmo.short_names.as_slice()),
        ("gizmos", ["gz".to_owned()].as_slice())
    );
    assert!(discovery.registry().get(&crd.gvk).is_some());
    assert!(
        discovery
            .resolve(&crd.gvk)
            .await
            .expect("resolve")
            .is_some()
    );

    crd.delete(&client).await;
    let diff = next_diff(&mut changes, "removal of the CRD's kind", |d| {
        d.removed.iter().any(|k| k.gvk == crd.gvk)
    })
    .await;
    assert!(diff.added.iter().all(|k| k.gvk != crd.gvk));
    assert!(discovery.registry().get(&crd.gvk).is_none());
}

#[tokio::test]
async fn a_resolve_miss_finds_a_crd_created_after_discovery() {
    let Some(ctx) = test_context() else { return };
    let client = client_for(&ctx).await;
    let discovery = KubeDiscovery::with_config(
        client.clone(),
        DiscoveryConfig {
            miss_cooldown: Duration::ZERO,
            ..DiscoveryConfig::default()
        },
    );
    discovery.discover().await.expect("discover");

    let crd = TestCrd::create(&client, &ctx).await;
    // A CRD is served a moment after creation (until it is established, discovery omits it),
    // so poll `resolve` until the deadline.
    let started = Instant::now();
    loop {
        if discovery
            .resolve(&crd.gvk)
            .await
            .expect("resolve")
            .is_some()
        {
            break;
        }
        assert!(
            started.elapsed() < DEADLINE,
            "CRD not served within {DEADLINE:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    crd.delete(&client).await;
}
