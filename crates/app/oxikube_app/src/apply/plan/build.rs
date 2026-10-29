//! Building the plan: the domain's [`ApplyPlan`] over the cluster's discovery, every object
//! sanitised for apply first. The service builds it to show; the handler builds it again from the
//! command's objects (an agent sends objects, not a plan), which changes nothing for a plan the
//! service built: the order is stable and sanitising and namespace defaulting are idempotent.

use std::collections::HashMap;

use oxikube_domain::apply::{ApplyPlan, SanitizeMode, sanitize_for_apply};
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::manifests::ManifestDoc;
use oxikube_ports::DiscoveryPort;

use crate::apply::manifest::DEFAULT_NAMESPACE;

/// `docs` sanitised ([`SanitizeMode::Apply`]: what the server owns goes, what the user wrote
/// stays), in kind order, every namespaced object without a namespace put in `namespace` (else
/// `default`). Whether a kind is namespaced comes from `discovery` (a kind it does not serve
/// takes the scope a CustomResourceDefinition in the plan gives it, else is left as written).
pub(crate) async fn build_plan(
    mut docs: Vec<ManifestDoc>,
    namespace: Option<&str>,
    discovery: Option<&dyn DiscoveryPort>,
) -> ApplyPlan {
    for doc in &mut docs {
        sanitize_for_apply(&mut doc.json, SanitizeMode::Apply);
    }
    let scopes = scopes(&docs, discovery).await;
    let namespace = namespace
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or(DEFAULT_NAMESPACE);
    ApplyPlan::build(docs, Some(namespace), |gvk| {
        scopes.get(gvk).copied().flatten()
    })
}

/// What discovery says about each kind of `docs` (`None`: it does not serve it, or could not
/// say): one lookup per distinct kind.
async fn scopes(
    docs: &[ManifestDoc],
    discovery: Option<&dyn DiscoveryPort>,
) -> HashMap<Gvk, Option<Scope>> {
    let mut scopes = HashMap::new();
    let Some(discovery) = discovery else {
        return scopes;
    };
    for doc in docs {
        let gvk = gvk_of(doc);
        if scopes.contains_key(&gvk) {
            continue;
        }
        let scope = discovery
            .resolve(&gvk)
            .await
            .ok()
            .flatten()
            .map(|kind| kind.scope());
        scopes.insert(gvk, scope);
    }
    scopes
}

fn gvk_of(doc: &ManifestDoc) -> Gvk {
    let text = |key: &str| {
        doc.json
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    };
    Gvk::from_api_version(text("apiVersion"), text("kind"))
}
