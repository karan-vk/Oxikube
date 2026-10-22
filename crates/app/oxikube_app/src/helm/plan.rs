//! [`ResourcePlan`]: the objects of one release revision as `ResourceRef`s.

use oxikube_domain::helm::{ManifestError, ManifestObject, ParsedManifest, ReleaseRef};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef, Scope};

use super::kinds::KindScopes;

/// One manifest object, mapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedResource {
    /// Where the object should live. For a kind the cluster does not serve, the manifest's own
    /// `apiVersion` and namespace, unchanged.
    pub reference: ResourceRef,
    /// The template the object was rendered from.
    pub source: Option<String>,
    /// The kind's scope from discovery; `None` when the cluster does not serve the kind.
    pub scope: Option<Scope>,
}

/// The mapped objects of a release revision. Pure data: build once per `(release, revision)`
/// and keep (see `HelmService::plan`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourcePlan {
    /// The release's live objects, in manifest order.
    pub resources: Vec<PlannedResource>,
    /// Hook objects, not part of the live resources.
    pub hooks: Vec<ManifestObject>,
    /// Documents that could not be read.
    pub errors: Vec<ManifestError>,
}

impl ResourcePlan {
    /// Maps a parsed manifest of `release`. A namespaced kind without `metadata.namespace` gets
    /// the release namespace; a cluster-scoped kind never has one.
    pub fn build(release: &ReleaseRef, parsed: &ParsedManifest, kinds: &KindScopes) -> Self {
        let resources = parsed
            .objects
            .iter()
            .map(|object| plan_one(&release.cluster, &release.namespace, object, kinds))
            .collect();
        Self {
            resources,
            hooks: parsed.hooks.clone(),
            errors: parsed.errors.clone(),
        }
    }
}

fn plan_one(
    cluster: &ClusterId,
    release_namespace: &str,
    object: &ManifestObject,
    kinds: &KindScopes,
) -> PlannedResource {
    let served = kinds.resolve(object.group(), object.version(), &object.kind);
    let (gvk, namespace, scope) = match served {
        Some(served) => {
            let namespace = match served.scope {
                Scope::Namespaced => Some(
                    object
                        .namespace
                        .clone()
                        .unwrap_or_else(|| release_namespace.to_owned()),
                ),
                Scope::Cluster => None,
            };
            (served.gvk, namespace, Some(served.scope))
        }
        None => (
            Gvk::from_api_version(&object.api_version, &object.kind),
            object.namespace.clone(),
            None,
        ),
    };
    PlannedResource {
        reference: ResourceRef::new(
            cluster.clone(),
            gvk,
            namespace.map(Into::into),
            object.name.as_str(),
        ),
        source: object.source.clone(),
        scope,
    }
}
