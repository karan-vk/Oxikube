//! The reference scan: what a copied object points at that the target does not have.
//!
//! A workload names a ServiceAccount, ConfigMaps, Secrets, claims and image pull secrets in its
//! pod spec. A copy that arrives without them starts pods that cannot start, so the review warns
//! (never blocks: copying dependencies is a separate feature).

use futures::future::join_all;
use oxikube_domain::Resource;
use oxikube_domain::view::{PodRef, pod_references};
use oxikube_ports::ResourceReader;
use serde_json::{Value, json};

use super::types::MissingReference;

/// The most references looked up; a pod spec names a handful.
const MAX_LOOKUPS: usize = 64;

/// The pod spec of `object`: a Pod's own, a workload's template's, a CronJob's job template's.
fn pod_spec(object: &Value) -> Option<&Value> {
    [
        "/spec/template/spec",
        "/spec/jobTemplate/spec/template/spec",
    ]
    .into_iter()
    .find_map(|pointer| object.pointer(pointer))
    .or_else(|| {
        (object.get("kind").and_then(Value::as_str) == Some("Pod"))
            .then(|| object.get("spec"))
            .flatten()
    })
}

/// The references of `object` (a body about to be written), empty for a kind without a pod spec.
pub(super) fn references(object: &Value) -> Vec<PodRef> {
    let Some(spec) = pod_spec(object) else {
        return Vec::new();
    };
    // `pod_references` reads a Pod; the template's spec is the same shape.
    let pod = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "scan"},
        "spec": spec,
    });
    Resource::from_json(pod)
        .map(|pod| pod_references(&pod))
        .unwrap_or_default()
}

/// The references of `object` that the target lacks in `namespace`. A lookup that fails (no
/// right to read that kind) says nothing and so warns of nothing.
pub(super) async fn missing(
    object: &Value,
    namespace: Option<&str>,
    reader: &dyn ResourceReader,
) -> Vec<MissingReference> {
    let wanted: Vec<PodRef> = references(object).into_iter().take(MAX_LOOKUPS).collect();
    let lookups = wanted.iter().map(|reference| async move {
        let namespace = if reference.kind.is_cluster_scoped() {
            None
        } else {
            namespace
        };
        let found = reader
            .get_opt(&reference.kind.gvk(), namespace, &reference.name)
            .await;
        matches!(found, Ok(None)).then(|| MissingReference {
            kind: reference.kind.title().to_owned(),
            name: reference.name.to_string(),
            namespace: namespace.map(str::to_owned),
        })
    });
    join_all(lookups).await.into_iter().flatten().collect()
}
