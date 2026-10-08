//! The objects of a synthetic cluster that do not change: what discovery serves, the namespaces,
//! nodes, workloads, services and ConfigMaps, and the 5 MB ConfigMap the detail drawer scenario
//! opens.

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_testkit::{deployment, node, replicaset};
use serde_json::{Map, Value, json};

use super::population::namespace_name;

/// The large ConfigMap's name (in the first namespace).
pub const BIG_CONFIG_MAP: &str = "big-config";
/// Its size: 5 MB of data.
pub const BIG_CONFIG_MAP_BYTES: usize = 5 * 1024 * 1024;
/// Nodes.
const NODES: usize = 6;

fn kind(group: &str, version: &str, name: &str, plural: &str, namespaced: bool) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, version, name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: vec!["all".into()],
        verbs: VerbSet::from_names([
            "get", "list", "watch", "create", "delete", "patch", "update",
        ]),
        namespaced,
    }
}

/// What discovery lists: the core kinds the sidebar, the overview and the scenarios use.
pub fn kinds() -> Vec<ResourceKind> {
    vec![
        kind("", "v1", "Pod", "pods", true),
        kind("", "v1", "Namespace", "namespaces", false),
        kind("", "v1", "Event", "events", true),
        kind("", "v1", "ConfigMap", "configmaps", true),
        kind("", "v1", "Service", "services", true),
        kind("", "v1", "Node", "nodes", false),
        kind("apps", "v1", "Deployment", "deployments", true),
        kind("apps", "v1", "ReplicaSet", "replicasets", true),
        kind("apps", "v1", "StatefulSet", "statefulsets", true),
        kind("apps", "v1", "DaemonSet", "daemonsets", true),
        kind("batch", "v1", "Job", "jobs", true),
    ]
}

/// Every fixed object of a cluster with `namespaces` namespaces, created two days before `now`.
pub fn objects(namespaces: usize, now: Timestamp) -> Vec<Resource> {
    let created = now
        .checked_sub(jiff::SignedDuration::from_hours(49))
        .unwrap_or(now)
        .to_string();
    let mut out = Vec::new();
    for n in 0..NODES {
        out.push(
            node()
                .name(format!("worker-{n}"))
                .ready()
                .created(created.clone())
                .build(),
        );
    }
    for i in 0..namespaces.max(1) {
        let ns = namespace_name(i);
        out.extend(from_json(json!({
            "apiVersion": "v1", "kind": "Namespace",
            "metadata": {"name": ns, "creationTimestamp": created, "labels": {"app": "oxikube-load"}},
            "status": {"phase": "Active"},
        })));
        out.push(
            deployment()
                .namespace(ns.clone())
                .name("load")
                .replicas(10)
                .ready(10)
                .created(created.clone())
                .build(),
        );
        out.push(
            replicaset()
                .namespace(ns.clone())
                .name("load-7d9c4b5f6")
                .replicas(10)
                .ready(10)
                .created(created.clone())
                .build(),
        );
        out.extend(from_json(json!({
            "apiVersion": "v1", "kind": "Service",
            "metadata": {"name": "load", "namespace": ns, "creationTimestamp": created},
            "spec": {"type": "ClusterIP", "clusterIP": format!("10.96.0.{}", i + 10),
                "ports": [{"port": 80, "protocol": "TCP"}]},
        })));
        out.extend(from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "kube-root-ca.crt", "namespace": ns, "creationTimestamp": created},
            "data": {"ca.crt": "-----BEGIN CERTIFICATE-----\nMIIC...\n-----END CERTIFICATE-----\n"},
        })));
    }
    out.extend(big_config_map(&created));
    out
}

/// A 5 MB ConfigMap in the first namespace: 1 280 keys of about 4 KiB of configuration text
/// each, so its YAML and its description are long, many-line documents.
fn big_config_map(created: &str) -> Option<Resource> {
    let mut data = Map::new();
    let mut size = 0;
    let mut k = 0;
    while size < BIG_CONFIG_MAP_BYTES {
        let mut text = String::with_capacity(4_200);
        for line in 0..52 {
            text.push_str(&format!(
                "service.{k}.route.{line}: upstream=payments-{line}.svc.cluster.local:8080 \
                 timeout=30s retries=3\n"
            ));
        }
        size += text.len();
        data.insert(format!("routes-{k:04}.properties"), Value::String(text));
        k += 1;
    }
    from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": BIG_CONFIG_MAP, "namespace": namespace_name(0),
            "creationTimestamp": created, "labels": {"app": "oxikube-load"}},
        "data": data,
    }))
}

/// Events about the large ConfigMap (the drawer's Events tab lists them), `count` of them.
pub fn big_config_map_events(count: usize, now: Timestamp) -> Vec<Resource> {
    (0..count)
        .filter_map(|i| {
            let at = now
                .checked_sub(jiff::SignedDuration::from_mins(
                    i64::try_from(i).unwrap_or(0) * 3,
                ))
                .unwrap_or(now)
                .to_string();
            from_json(json!({
                "apiVersion": "v1", "kind": "Event",
                "metadata": {"name": format!("{BIG_CONFIG_MAP}.{i:04x}"),
                    "namespace": namespace_name(0), "creationTimestamp": at,
                    "resourceVersion": format!("{}", 1_000_000 + i)},
                "involvedObject": {"apiVersion": "v1", "kind": "ConfigMap",
                    "name": BIG_CONFIG_MAP, "namespace": namespace_name(0)},
                "reason": if i % 3 == 0 { "Updated" } else { "Synced" },
                "message": format!("config revision {} rolled out to {} replicas", 400 - i, 10),
                "type": "Normal", "count": i + 1,
                "firstTimestamp": at, "lastTimestamp": at,
                "source": {"component": "config-controller"},
            }))
        })
        .collect()
}

fn from_json(json: Value) -> Option<Resource> {
    Resource::from_json(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cluster_has_its_namespaces_nodes_and_the_big_config_map() {
        let now: Timestamp = "2026-10-08T12:00:00Z".parse().unwrap();
        let objects = objects(8, now);
        let count = |kind: &str| objects.iter().filter(|o| &*o.kind.kind == kind).count();
        assert_eq!(count("Namespace"), 8);
        assert_eq!(count("Node"), NODES);
        assert_eq!(count("Deployment"), 8);
        assert_eq!(count("ConfigMap"), 9);
        let big = objects
            .iter()
            .find(|o| o.name() == BIG_CONFIG_MAP)
            .expect("the big ConfigMap");
        let size = serde_json::to_string(&big.json).unwrap().len();
        assert!(
            (BIG_CONFIG_MAP_BYTES..BIG_CONFIG_MAP_BYTES + 200_000).contains(&size),
            "{size}"
        );
        assert!(kinds().iter().any(|k| &*k.gvk.kind == "Event"));
    }
}
