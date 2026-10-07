//! A CRD created while a session is connected reaches the session's update stream and the
//! session's discovery, with no reconnect (E03-F544): the manager starts the adapter's CRD watch
//! through `DiscoveryPort::subscribe` when the connection is established.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Api;
use kube::api::{DeleteParams, PostParams};
use oxikube_app::session::{ClusterSessionManager, SessionChange, SessionOptions};
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::SessionPhase;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::FakeClusterSourcePort;

use crate::DEADLINE;
use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Deletes the CRD (best effort, no wait) when dropped, also when the test panics.
struct Cleanup {
    context: String,
    name: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new("kubectl")
            .args(["--context", &self.context, "delete", "crd", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}

/// Waits for a `KindsChanged` update of the session for which `wanted` holds.
async fn next_kinds(
    updates: &mut oxikube_app::session::SessionUpdates,
    what: &str,
    wanted: impl Fn(&oxikube_ports::KindsChange) -> bool,
) {
    use futures::StreamExt as _;
    let started = Instant::now();
    loop {
        let remaining = DEADLINE.saturating_sub(started.elapsed());
        match tokio::time::timeout(remaining, updates.next()).await {
            Ok(Some(Ok(update))) => {
                if let SessionChange::KindsChanged(change) = update.change {
                    if wanted(&change) {
                        return;
                    }
                }
            }
            Ok(Some(Err(_))) => {}
            Ok(None) => panic!("update stream ended while waiting for {what}"),
            Err(_) => panic!("{what} not seen within {DEADLINE:?}"),
        }
    }
}

#[tokio::test]
async fn a_crd_created_while_connected_reaches_the_session_without_reconnecting() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let connector = KubeConnector::new(
        kind.kubeconfig.clone(),
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let manager = ClusterSessionManager::new(
        Arc::new(connector),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
    );
    let entry = catalog_entry(&kind.context);
    manager.open(&entry, SessionOptions::default());
    let mut updates = manager.subscribe();
    let state = manager.connect(&entry.cluster).await.expect("connect");
    assert_eq!(state.phase(), SessionPhase::Ready, "{state:?}");

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let suffix = std::process::id() ^ nanos;
    let group = format!("smoke-{suffix:x}.test.oxikube.dev");
    let name = format!("sprockets.{group}");
    let _cleanup = Cleanup {
        context: kind.context.to_string(),
        name: name.clone(),
    };
    let crd: CustomResourceDefinition = serde_json::from_value(serde_json::json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {"name": name},
        "spec": {
            "group": group,
            "scope": "Namespaced",
            "names": {"plural": "sprockets", "singular": "sprocket", "kind": "Sprocket",
                      "listKind": "SprocketList"},
            "versions": [{
                "name": "v1", "served": true, "storage": true,
                "schema": {"openAPIV3Schema": {"type": "object",
                           "x-kubernetes-preserve-unknown-fields": true}},
            }],
        },
    }))
    .expect("crd json");
    let client = kind.admin_client().await;
    let crds = Api::<CustomResourceDefinition>::all(client);
    let gvk = Gvk::new(group.as_str(), "v1", "Sprocket");

    crds.create(&PostParams::default(), &crd)
        .await
        .expect("create CRD");
    next_kinds(&mut updates, "the new CRD's kind", |c| {
        c.added.contains(&gvk)
    })
    .await;
    let session = manager.get(&entry.cluster).expect("session");
    assert!(session.is_connected(), "no reconnect happened");
    let discovery = session.discovery().expect("connected");
    assert!(
        discovery.resolve(&gvk).await.expect("resolve").is_some(),
        "the session's discovery serves the new kind"
    );

    crds.delete(&name, &DeleteParams::default())
        .await
        .expect("delete CRD");
    next_kinds(&mut updates, "removal of the CRD's kind", |c| {
        c.removed.contains(&gvk)
    })
    .await;
    assert!(
        discovery.resolve(&gvk).await.expect("resolve").is_none(),
        "the removed kind is gone from the registry"
    );
    // Disconnecting stops the watch; nothing more arrives for the session.
    manager.disconnect(&entry.cluster).expect("disconnect");
    tokio::time::sleep(Duration::from_millis(200)).await;
}
