//! Fixtures for the resource data plane scenarios (E04): the adapter under test and pods
//! that cost the cluster nothing.

use std::collections::BTreeMap;

use futures::{StreamExt, stream};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
use kube::api::{ObjectMeta, PostParams};
use kube::{Api, Client};
use oxikube_kube::{KubeDiscovery, KubeResources, ResourcesConfig};

/// The adapter on `client`, with discovery on the same client.
pub fn adapter(client: &Client) -> KubeResources {
    adapter_with(client, ResourcesConfig::default())
}

/// As [`adapter`] with explicit settings.
pub fn adapter_with(client: &Client, config: ResourcesConfig) -> KubeResources {
    KubeResources::with_config(client.clone(), KubeDiscovery::new(client.clone()), config)
}

/// A pod that cannot be scheduled (`nodeSelector` no node has), so it stays `Pending`:
/// thousands of them load the API server and etcd but not the kubelet.
pub fn pending_pod(name: &str, labels: &[(&str, &str)]) -> Pod {
    pod(name, labels, None)
}

/// A pod bound to `node` (`spec.nodeName`), bypassing the scheduler.
pub fn bound_pod(name: &str, labels: &[(&str, &str)], node: &str) -> Pod {
    pod(name, labels, Some(node))
}

fn pod(name: &str, labels: &[(&str, &str)], node: Option<&str>) -> Pod {
    let labels: BTreeMap<String, String> = labels
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    Pod {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            labels: Some(labels),
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            node_name: node.map(str::to_owned),
            node_selector: node.is_none().then(|| {
                BTreeMap::from([("oxikube.test/unschedulable".to_owned(), "true".to_owned())])
            }),
            termination_grace_period_seconds: Some(1),
            containers: vec![Container {
                name: "pause".into(),
                image: Some("registry.k8s.io/pause:3.10".into()),
                ..Container::default()
            }],
            ..PodSpec::default()
        }),
        ..Pod::default()
    }
}

/// Creates `pods` in `namespace`, 64 requests at a time.
pub async fn create_pods(client: &Client, namespace: &str, pods: Vec<Pod>) {
    let api = Api::<Pod>::namespaced(client.clone(), namespace);
    let results: Vec<_> = stream::iter(pods)
        .map(|pod| {
            let api = api.clone();
            async move { api.create(&PostParams::default(), &pod).await }
        })
        .buffer_unordered(64)
        .collect()
        .await;
    for result in results {
        result.expect("create pod");
    }
}

/// The first JSON pointer at which `a` and `b` differ, with both values, or `None` when equal.
/// Failure output for the golden comparison: whole-object diffs of a pod are unreadable.
pub fn first_difference(a: &serde_json::Value, b: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    fn walk(a: &Value, b: &Value, path: &str) -> Option<String> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for key in x.keys().chain(y.keys()) {
                    let at = format!("{path}/{key}");
                    match (x.get(key), y.get(key)) {
                        (Some(l), Some(r)) => {
                            if let Some(found) = walk(l, r, &at) {
                                return Some(found);
                            }
                        }
                        (l, r) => return Some(format!("{at}: {l:?} vs {r:?}")),
                    }
                }
                None
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
                .iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (l, r))| walk(l, r, &format!("{path}/{i}"))),
            _ if a == b => None,
            _ => Some(format!("{path}: {a} vs {b}")),
        }
    }
    walk(a, b, "")
}

/// `value` with every `null` object member removed, recursively. The typed path cannot carry
/// an explicit `null` (an unset `Option` serialises as absent), the dynamic path keeps what
/// the server sent (`lastProbeTime: null`), and the API treats the two alike for core kinds.
pub fn without_nulls(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), without_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_nulls).collect()),
        other => other.clone(),
    }
}
