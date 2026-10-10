//! The `:` jump bar's alias table against a real cluster (E11-S04): connecting fills it from the
//! API server's discovery, a CRD created or deleted while connected updates only its names, and
//! the built-ins follow the versions the server prefers.

use std::process::Command;
use std::sync::Arc;

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Api;
use kube::api::{DeleteParams, PostParams};
use oxikube_app::session::{ClusterSessionManager, SessionOptions};
use oxikube_app::{AliasRegistry, AliasSource, Resolution};
use oxikube_domain::AliasTarget;
use oxikube_domain::session::SessionPhase;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::FakeClusterSourcePort;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};
use crate::eventually;

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

#[tokio::test]
async fn the_table_follows_the_clusters_discovery_and_its_crds() {
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

    let registry = AliasRegistry::new();
    let _follow = registry.follow(&manager, &tokio::runtime::Handle::current());
    let table = registry.table(&entry.cluster);
    assert!(
        table.resolve("dp").is_known(),
        "built-ins are there before connecting"
    );
    assert!(
        !table.resolve("events.events.k8s.io").is_known(),
        "discovery has not answered yet"
    );

    let state = manager.connect(&entry.cluster).await.expect("connect");
    assert_eq!(state.phase(), SessionPhase::Ready, "{state:?}");
    eventually(
        "the table to learn the cluster's kinds",
        || format!("{table:?}"),
        || {
            let known = table.resolve("events.events.k8s.io").is_known();
            async move { known }
        },
    )
    .await;

    // The built-in follows the version the server serves, and discovery's own words work too.
    let Resolution::Exact(dp) = table.resolve("dp") else {
        panic!("dp is exact");
    };
    assert_eq!(dp.source, AliasSource::BuiltIn);
    let AliasTarget::Gvr(gvr) = &dp.target else {
        panic!("dp leads to a resource");
    };
    assert_eq!(
        (&*gvr.group, &*gvr.version, &*gvr.resource),
        ("apps", "v1", "deployments")
    );
    assert!(
        table
            .resolve("customresourcedefinitions.apiextensions.k8s.io")
            .is_known()
    );

    // A CRD with a short name, created while connected.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let suffix = std::process::id() ^ nanos;
    let group = format!("alias-{suffix:x}.test.oxikube.dev");
    let name = format!("cogs.{group}");
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
            "names": {"plural": "cogs", "singular": "cog", "kind": "Cog",
                      "listKind": "CogList", "shortNames": ["cg"]},
            "versions": [{
                "name": "v1", "served": true, "storage": true,
                "schema": {"openAPIV3Schema": {"type": "object",
                           "x-kubernetes-preserve-unknown-fields": true}},
            }],
        },
    }))
    .expect("crd json");
    let crds = Api::<CustomResourceDefinition>::all(kind.admin_client().await);
    crds.create(&PostParams::default(), &crd)
        .await
        .expect("create CRD");

    eventually(
        "the CRD's short name to resolve",
        || format!("{table:?}"),
        || {
            let known = table.resolve("cg").is_known();
            async move { known }
        },
    )
    .await;
    for word in ["cogs", "cog", "CG", &format!("cogs.{group}")] {
        let Resolution::Exact(entry) = table.resolve(word) else {
            panic!("`{word}` should be exact");
        };
        let AliasTarget::Gvr(gvr) = &entry.target else {
            panic!("`{word}` should lead to a resource");
        };
        assert_eq!(
            (&*gvr.group, &*gvr.version, &*gvr.resource),
            (group.as_str(), "v1", "cogs")
        );
        assert_eq!(entry.source, AliasSource::Discovery);
    }

    crds.delete(&name, &DeleteParams::default())
        .await
        .expect("delete CRD");
    eventually(
        "the CRD's names to go",
        || format!("{table:?}"),
        || {
            let gone = !table.resolve("cg").is_known();
            async move { gone }
        },
    )
    .await;
    assert!(table.resolve("dp").is_known(), "the rest stays");

    // Disconnecting drops what discovery added and keeps the built-ins.
    manager.disconnect(&entry.cluster).expect("disconnect");
    eventually(
        "discovery's names to go on disconnect",
        || format!("{table:?}"),
        || {
            let gone = !table.resolve("events.events.k8s.io").is_known();
            async move { gone }
        },
    )
    .await;
    assert!(table.resolve("dp").is_known());
}
