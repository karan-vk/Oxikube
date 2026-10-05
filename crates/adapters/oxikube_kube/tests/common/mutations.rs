//! Fixtures for the mutation scenarios (E04-S05): ConfigMap manifests, which cost the cluster
//! nothing and carry every field the scenarios need (data, labels, owner references).

use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiResult, Resource};
use oxikube_kube::KubeResources;
use oxikube_ports::ResourceReader;
use oxikube_testkit::integration::TestNamespace;
use serde_json::{Value, json};

use super::resources::adapter;

/// The adapter on the kind cluster and a fresh namespace, or `None` to skip.
pub async fn setup() -> Option<(KubeResources, TestNamespace)> {
    let kind = super::kind().await?;
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    Some((adapter(&client), ns))
}

/// `v1/ConfigMap`.
pub fn configmap_gvk() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

/// A ConfigMap manifest as a caller sends it (no `apiVersion`/`kind`): `data` is `{key: value}`.
pub fn configmap(name: &str, labels: &[(&str, &str)], data: &[(&str, &str)]) -> Value {
    let pairs = |pairs: &[(&str, &str)]| -> Value {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), json!(v)))
            .collect()
    };
    json!({"metadata": {"name": name, "labels": pairs(labels)}, "data": pairs(data)})
}

/// A ConfigMap manifest owned by `owner` (blocking its foreground deletion).
pub fn owned_configmap(name: &str, owner: &Resource) -> Value {
    let mut manifest = configmap(name, &[], &[]);
    manifest["metadata"]["ownerReferences"] = json!([{
        "apiVersion": "v1", "kind": "ConfigMap", "name": owner.name(),
        "uid": owner.meta.uid, "blockOwnerDeletion": true,
    }]);
    manifest
}

/// The live ConfigMap `name`, or `None`.
pub async fn live(
    resources: &KubeResources,
    namespace: &str,
    name: &str,
) -> OxiResult<Option<Resource>> {
    resources
        .get_opt(&configmap_gvk(), Some(namespace), name)
        .await
}

/// The field managers recorded in `managedFields` of `object`.
pub fn managers(object: &Resource) -> Vec<String> {
    object.json["metadata"]["managedFields"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e["manager"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
