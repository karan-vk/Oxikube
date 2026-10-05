//! Fixtures for the subresource scenarios (E04-S06): workloads that cost the cluster nothing,
//! a running pod, and a CRD that declares `status` and `scale` subresources.
//!
//! Everything namespaced lives in the test's `oxi-test-<rand>` namespace. Cluster-scoped
//! objects (the CRD, a tainted fake Node) carry a random suffix and clean up on drop.

use std::process::Command;
use std::sync::Arc;

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{DeleteParams, PostParams};
use kube::{Api, Client};
use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiResult, Resource};
use oxikube_kube::{KubeResources, PoolConfig, RetryMode};
use oxikube_ports::ResourceReader;
use oxikube_testkit::integration::TestNamespace;
use serde_json::{Value, json};

use super::resources::adapter;

/// The pause image the kind node already has.
pub const PAUSE: &str = "registry.k8s.io/pause:3.10";

/// The kind cluster, the adapter on it and a fresh namespace.
pub struct Env {
    /// The adapter under test.
    pub resources: KubeResources,
    /// The admin client, for fixtures the adapter does not create.
    pub client: Arc<Client>,
    /// The test's namespace, deleted on drop.
    pub ns: TestNamespace,
    /// The kind kubectl context.
    pub context: String,
}

impl Env {
    /// The namespace name.
    pub fn namespace(&self) -> &str {
        self.ns.name()
    }
}

/// The environment, or `None` to skip the test (`OXIKUBE_TEST_CONTEXT` unset).
pub async fn setup() -> Option<Env> {
    let kind = super::kind().await?;
    let context = kind.context.as_str().to_owned();
    let ns = TestNamespace::create(&context).expect("test namespace");
    let client = kind.admin_client().await;
    // Evictions need the 429 of a blocking budget at once, not after the pool's retries.
    let unretried = kind
        .pool_with(
            kind.kubeconfig.clone(),
            PoolConfig {
                retry: RetryMode::Disabled,
                ..PoolConfig::default()
            },
        )
        .get(&kind.context)
        .await
        .expect("unretried client");
    Some(Env {
        resources: adapter(&client).with_unretried_client((*unretried).clone()),
        client,
        ns,
        context,
    })
}

/// `apps/v1/Deployment`.
pub fn deployment_gvk() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

/// `v1/Pod`.
pub fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// `v1/Node`.
pub fn node_gvk() -> Gvk {
    Gvk::new("", "v1", "Node")
}

/// `batch/v1/CronJob`.
pub fn cronjob_gvk() -> Gvk {
    Gvk::new("batch", "v1", "CronJob")
}

/// A Deployment of pause containers with `replicas` replicas (a manifest, no `apiVersion`).
pub fn deployment(name: &str, replicas: i32) -> Value {
    json!({
        "metadata": {"name": name},
        "spec": {
            "replicas": replicas,
            "selector": {"matchLabels": {"app": name}},
            "template": {
                "metadata": {"labels": {"app": name}},
                "spec": {
                    "terminationGracePeriodSeconds": 1,
                    "containers": [{"name": "pause", "image": PAUSE}],
                },
            },
        },
    })
}

/// A pod running the pause image, with `labels` (a manifest, no `apiVersion`). The pod has CPU
/// and memory requests so a resize has something to change.
pub fn running_pod(name: &str, labels: &[(&str, &str)]) -> Value {
    let labels: serde_json::Map<String, Value> = labels
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect();
    json!({
        "metadata": {"name": name, "labels": labels},
        "spec": {
            "terminationGracePeriodSeconds": 1,
            "containers": [{
                "name": "pause", "image": PAUSE,
                "resources": {"requests": {"cpu": "10m", "memory": "8Mi"},
                              "limits": {"cpu": "50m"}},
            }],
        },
    })
}

/// A CronJob that never fires (`schedule` far in the future) and runs nothing when it does.
pub fn cronjob(name: &str) -> Value {
    json!({
        "metadata": {"name": name},
        "spec": {
            "schedule": "0 0 29 2 1",
            "jobTemplate": {"spec": {"template": {"spec": {
                "restartPolicy": "Never",
                "containers": [{"name": "pause", "image": PAUSE}],
            }}}},
        },
    })
}

/// The live pod `name`, or `None`.
pub async fn live_pod(
    resources: &KubeResources,
    namespace: &str,
    name: &str,
) -> OxiResult<Option<Resource>> {
    resources.get_opt(&pod_gvk(), Some(namespace), name).await
}

/// Whether the pod reports `Ready=True`.
pub fn is_ready(pod: &Resource) -> bool {
    pod.json["status"]["conditions"]
        .as_array()
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|c| c["type"] == "Ready" && c["status"] == "True")
        })
}

/// A random lowercase suffix for cluster-scoped names.
pub fn suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_owned()
}

/// A CRD `widgets.rt-<rand>.test.oxikube.dev` (kind `Widget`) with `status` and `scale`
/// subresources, or without when built by [`SubCrd::create_plain`]. Deleted (best effort)
/// when dropped.
pub struct SubCrd {
    context: String,
    name: String,
    /// `<group>/v1 Widget`.
    pub gvk: Gvk,
}

impl SubCrd {
    /// A CRD with `status` and `scale` subresources.
    pub async fn create(client: &Client, context: &str) -> Self {
        Self::build(client, context, true).await
    }

    /// A CRD with no subresources at all.
    pub async fn create_plain(client: &Client, context: &str) -> Self {
        Self::build(client, context, false).await
    }

    async fn build(client: &Client, context: &str, subresources: bool) -> Self {
        let group = format!("rt-{}.test.oxikube.dev", suffix());
        let name = format!("widgets.{group}");
        let schema = json!({"openAPIV3Schema": {
            "type": "object",
            "properties": {
                "spec": {"type": "object", "properties": {"size": {"type": "integer"}}},
                "status": {"type": "object", "properties": {
                    "phase": {"type": "string"}, "size": {"type": "integer"},
                }},
            },
        }});
        let mut version = json!({"name": "v1", "served": true, "storage": true, "schema": schema});
        if subresources {
            version["subresources"] = json!({
                "status": {},
                "scale": {"specReplicasPath": ".spec.size", "statusReplicasPath": ".status.size"},
            });
        }
        let crd: CustomResourceDefinition = serde_json::from_value(json!({
            "apiVersion": "apiextensions.k8s.io/v1",
            "kind": "CustomResourceDefinition",
            "metadata": {"name": name},
            "spec": {
                "group": group, "scope": "Namespaced",
                "names": {"plural": "widgets", "singular": "widget", "kind": "Widget",
                          "listKind": "WidgetList"},
                "versions": [version],
            },
        }))
        .expect("crd json");
        // Registered before the create so a failed or interrupted create still cleans up.
        let guard = Self {
            context: context.to_owned(),
            name,
            gvk: Gvk::new(group, "v1", "Widget"),
        };
        Api::<CustomResourceDefinition>::all(client.clone())
            .create(&PostParams::default(), &crd)
            .await
            .expect("create CRD");
        guard
    }

    /// Deletes the CRD now.
    pub async fn delete(&self, client: &Client) {
        Api::<CustomResourceDefinition>::all(client.clone())
            .delete(&self.name, &DeleteParams::default())
            .await
            .expect("delete CRD");
    }
}

impl Drop for SubCrd {
    fn drop(&mut self) {
        let _ = Command::new("kubectl")
            .args(["--context", &self.context, "delete", "crd", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}

/// A Node object that no kubelet backs, created tainted `NoSchedule` so the scheduler never
/// places a pod on it, and deleted on drop. Cordon scenarios run on it, never on the real node.
pub struct FakeNode {
    context: String,
    /// The node's name (`oxi-fake-<rand>`).
    pub name: String,
}

impl FakeNode {
    /// Creates the node through `resources`' client.
    pub async fn create(client: &Client, context: &str) -> Self {
        let name = format!("oxi-fake-{}", suffix());
        let node: k8s_openapi::api::core::v1::Node = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Node",
            "metadata": {"name": name},
            "spec": {"taints": [{"key": "oxikube.test/fake", "effect": "NoSchedule"}]},
        }))
        .expect("node json");
        let guard = Self {
            context: context.to_owned(),
            name,
        };
        Api::<k8s_openapi::api::core::v1::Node>::all(client.clone())
            .create(&PostParams::default(), &node)
            .await
            .expect("create fake node");
        guard
    }
}

impl Drop for FakeNode {
    fn drop(&mut self) {
        let _ = Command::new("kubectl")
            .args(["--context", &self.context, "delete", "node", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}
