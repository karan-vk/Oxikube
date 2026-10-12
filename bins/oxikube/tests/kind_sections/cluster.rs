//! The cluster side: seeding the matrix's kinds, reading an object back, and the server's Table
//! columns. Every call runs on a Tokio runtime of its own (`kind_common::on_cluster`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kube::api::{
    Api, ApiResource, DeleteParams, DynamicObject, GroupVersionKind, ListParams, PostParams,
};
use oxikube_testkit::sections::{Api as Served, KindCase, Seed};

use crate::kind_common::{DEADLINE, on_cluster};

/// `case`'s API, for kube's dynamic client.
pub fn api_resource(case: &KindCase) -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk(case.group, case.version, case.kind),
        case.plural,
    )
}

fn api(client: kube::Client, case: &KindCase, namespace: &str) -> Api<DynamicObject> {
    let resource = api_resource(case);
    if case.namespaced() {
        Api::namespaced_with(client, namespace, &resource)
    } else {
        Api::all_with(client, &resource)
    }
}

/// Whether the cluster serves `case`'s kind (an optional API may be missing).
async fn served(client: &kube::Client, case: &KindCase) -> bool {
    if case.api == Served::BuiltIn {
        return true;
    }
    client
        .list_api_group_resources(&format!("{}/{}", case.group, case.version))
        .await
        .is_ok_and(|list| list.resources.iter().any(|r| r.kind == case.kind))
}

/// The seeded objects: the name of each kind's object, and the cluster-scoped ones to delete
/// when the test ends (the test's namespace takes the namespaced ones with it).
pub struct Seeded {
    context: String,
    names: HashMap<&'static str, String>,
    cluster_scoped: Vec<(ApiResource, String)>,
}

impl Seeded {
    /// The name of `case`'s object, `None` when the cluster does not serve the kind.
    pub fn name_of(&self, case: &KindCase) -> Option<String> {
        self.names.get(case.kind).cloned()
    }
}

impl Drop for Seeded {
    fn drop(&mut self) {
        let made = std::mem::take(&mut self.cluster_scoped);
        on_cluster(&self.context, async |client| {
            for (resource, name) in made {
                let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
                api.delete(&name, &DeleteParams::default()).await.ok();
            }
        });
    }
}

/// Creates the seed of every case the cluster serves, in order (a later seed may name an earlier
/// one). The Node case is the cluster's first node and the Namespace case the test's namespace:
/// neither is created.
pub fn seed(context: &str, seed: &Seed<'_>, cases: &[&KindCase]) -> Seeded {
    let mut seeded = Seeded {
        context: context.to_owned(),
        names: HashMap::new(),
        cluster_scoped: Vec::new(),
    };
    on_cluster(context, async |client| {
        for case in cases {
            if !served(&client, case).await {
                continue;
            }
            if case.kind == "Node" {
                let nodes = api(client.clone(), case, "")
                    .list(&ListParams::default().limit(1))
                    .await
                    .expect("list the nodes");
                let node = nodes.items[0].metadata.name.clone().expect("a node name");
                seeded.names.insert(case.kind, node);
                continue;
            }
            let name = case.seed_name(seed);
            if case.seeded_on_cluster() {
                let object: DynamicObject =
                    serde_json::from_value(case.seed(seed)).expect("a seed manifest");
                api(client.clone(), case, seed.namespace)
                    .create(&PostParams::default(), &object)
                    .await
                    .unwrap_or_else(|e| panic!("create the {} seed: {e}", case.kind));
                if case.kind == "Pod" {
                    wait_running(&api(client.clone(), case, seed.namespace), &name).await;
                }
                if !case.namespaced() {
                    seeded
                        .cluster_scoped
                        .push((api_resource(case), name.clone()));
                }
            }
            seeded.names.insert(case.kind, name);
        }
    });
    seeded
}

/// What a user's action would change of `case`'s object `name`, as the API server has it now:
/// its generation, deletion, labels, annotations and every field but `status` (which the
/// cluster's controllers and the kubelet keep writing on their own).
pub fn fingerprint(context: &str, case: &KindCase, namespace: &str, name: &str) -> String {
    on_cluster(context, async |client| {
        let object = api(client, case, namespace)
            .get(name)
            .await
            .unwrap_or_else(|e| panic!("get the {} {name}: {e}", case.kind));
        let mut data = object.data.clone();
        if let Some(fields) = data.as_object_mut() {
            fields.remove("status");
        }
        let meta = &object.metadata;
        serde_json::json!({
            "generation": meta.generation,
            "deletion": meta.deletion_timestamp,
            "labels": meta.labels,
            "annotations": meta.annotations,
            "data": data,
        })
        .to_string()
    })
}

/// Waits until the seeded Pod `name` runs: from then on only its status changes on its own.
async fn wait_running(pods: &Api<DynamicObject>, name: &str) {
    let started = Instant::now();
    loop {
        let pod = pods.get(name).await.expect("the seeded pod");
        if pod.data["status"]["phase"] == "Running" {
            return;
        }
        assert!(
            started.elapsed() < DEADLINE,
            "the seeded pod did not start: {}",
            pod.data["status"]
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The columns the API server prints for `path` (a list URL), as `kubectl get` shows them: the
/// Table's column definitions in order, a column of priority above 0 (shown with `-o wide`)
/// suffixed `*`.
pub fn server_columns(context: &str, path: &str) -> Vec<String> {
    on_cluster(context, async |client| {
        let request = http::Request::get(path)
            .header(
                http::header::ACCEPT,
                "application/json;as=Table;v=v1;g=meta.k8s.io",
            )
            .body(Vec::new())
            .expect("a request");
        let table: serde_json::Value = client.request(request).await.expect("the server's table");
        table["columnDefinitions"]
            .as_array()
            .expect("column definitions")
            .iter()
            .map(|column| {
                let name = column["name"].as_str().unwrap_or_default();
                let wide = column["priority"].as_i64().unwrap_or(0) > 0;
                format!("{name}{}", if wide { "*" } else { "" })
            })
            .collect()
    })
}

/// Creates a Widget (the sample CRD `cargo xtask kind-up` installs) named `name` in `namespace`;
/// `false` when the cluster does not serve Widgets.
pub fn create_widget(context: &str, namespace: &str, name: &str) -> bool {
    on_cluster(context, async |client| {
        let resource = ApiResource::from_gvk_with_plural(
            &GroupVersionKind::gvk("test.oxikube.dev", "v1", "Widget"),
            "widgets",
        );
        let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &resource);
        let widget: DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
            "metadata": {"name": name},
            "spec": {"size": "small", "replicas": 1, "owner": "sections"},
        }))
        .expect("a widget");
        api.create(&PostParams::default(), &widget).await.is_ok()
    })
}
