//! The cluster's side of the suite: the objects the tests start from, written straight to the API
//! server (not through the app), and what the server says about them afterwards.

use kube::api::{Patch, PatchParams};
use oxikube_testkit::images;
use serde_json::{Value, json};

use crate::kind_common::editor::dynamic_api;
use crate::kind_common::on_cluster;

/// The scheduler no cluster runs: the Deployments' pods stay `Pending` and cost the node nothing.
const NO_SCHEDULER: &str = "oxikube-test-no-scheduler";

/// The field manager of the objects the tests start from: neither `oxikube` nor `kubectl`.
pub const SETUP_MANAGER: &str = "oxi-test-setup";

const DEPLOYMENTS: (&str, &str, &str, &str) = ("apps", "v1", "Deployment", "deployments");

/// Creates the Deployment `name` in `namespace` with a server-side apply under [`SETUP_MANAGER`]
/// that leaves `spec.replicas` out: the server defaults it to 1 and no manager owns it (a plain
/// create would own the defaults too). Its pods name a scheduler nobody runs.
pub fn create_deployment(context: &str, namespace: &str, name: &str) {
    let object = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": { "name": name, "namespace": namespace },
        "spec": {
            "selector": { "matchLabels": { "app": name } },
            "template": {
                "metadata": { "labels": { "app": name } },
                "spec": {
                    "schedulerName": NO_SCHEDULER,
                    "terminationGracePeriodSeconds": 1,
                    "containers": [{ "name": "main", "image": images::PAUSE }],
                },
            },
        },
    });
    on_cluster(context, async |client| {
        dynamic_api(client, namespace, DEPLOYMENTS)
            .patch(
                name,
                &PatchParams::apply(SETUP_MANAGER),
                &Patch::Apply(&object),
            )
            .await
            .expect("create the deployment");
    });
}

/// `kubectl apply --server-side` of `spec.replicas: replicas` on the Deployment: the field
/// manager `kubectl` takes the field.
pub fn kubectl_apply_replicas(context: &str, namespace: &str, name: &str, replicas: u32) {
    let patch = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": { "name": name },
        "spec": { "replicas": replicas },
    });
    on_cluster(context, async |client| {
        dynamic_api(client, namespace, DEPLOYMENTS)
            .patch(name, &PatchParams::apply("kubectl"), &Patch::Apply(&patch))
            .await
            .expect("kubectl's server-side apply");
    });
}

/// The live Deployment as JSON.
pub fn deployment(context: &str, namespace: &str, name: &str) -> Value {
    on_cluster(context, async |client| {
        let object = dynamic_api(client, namespace, DEPLOYMENTS)
            .get(name)
            .await
            .expect("the deployment");
        serde_json::to_value(object).expect("JSON")
    })
}

/// `(manager, operation)` of each `managedFields` entry of `object` that owns the field at
/// `path` (`["spec", "replicas"]`).
pub fn owners(object: &Value, path: &[&str]) -> Vec<(String, String)> {
    let entries = object
        .pointer("/metadata/managedFields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    entries
        .iter()
        .filter(|entry| {
            let mut fields = &entry["fieldsV1"];
            path.iter().all(|key| {
                fields = &fields[format!("f:{key}")];
                !fields.is_null()
            })
        })
        .map(|entry| {
            let text = |key: &str| entry[key].as_str().unwrap_or_default().to_owned();
            (text("manager"), text("operation"))
        })
        .collect()
}

/// `text` (an object as the editor shows it) with the root `spec.replicas` set to `replicas`:
/// the first `  replicas:` line after the root `spec:` (the status's own come after it).
pub fn with_spec_replicas(text: &str, replicas: u32) -> String {
    let spec = text.find("\nspec:\n").expect("a root spec") + 1;
    let at = spec + text[spec..].find("\n  replicas: ").expect("spec.replicas") + 1;
    let end = at + text[at..].find('\n').expect("a line");
    format!("{}  replicas: {replicas}{}", &text[..at], &text[end..])
}
