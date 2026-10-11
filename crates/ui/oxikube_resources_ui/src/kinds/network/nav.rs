//! Following a reference to another object: the Service behind an Ingress or a route, the
//! Gateway a route attaches to, the GatewayClass of a Gateway.
//!
//! A link opens the target's detail (`resource::Open`). Core Services are `v1`; every other kind
//! (the Gateway API's) is resolved through discovery, which says which version the cluster
//! serves, so nothing here hard-codes `v1` or `v1alpha2`.

use std::sync::Arc;

use oxikube_domain::ids::{Gvk, ResourceRef, Scope};
use oxikube_domain::kinds::ResourceKind;

/// The kinds a cluster serves, as far as discovery has said.
#[derive(Clone, Default)]
pub(super) struct NavKinds {
    kinds: Option<Arc<[ResourceKind]>>,
}

impl NavKinds {
    pub(super) fn new(kinds: Vec<ResourceKind>) -> Self {
        Self {
            kinds: Some(kinds.into()),
        }
    }

    /// Whether discovery has answered.
    pub(super) fn is_known(&self) -> bool {
        self.kinds.is_some()
    }

    /// The version and scope `group`/`kind` is served at, `None` until discovery answers or when
    /// the cluster does not serve it. A core Service needs no discovery.
    pub(super) fn resolve(&self, group: &str, kind: &str) -> Option<(Gvk, Scope)> {
        if group.is_empty() && kind == "Service" {
            return Some((Gvk::new("", "v1", "Service"), Scope::Namespaced));
        }
        let kinds = self.kinds.as_deref()?;
        let named = |k: &&ResourceKind| *k.gvk.group == *group && *k.gvk.kind == *kind;
        let found = kinds
            .iter()
            .filter(named)
            .find(|k| k.preferred)
            .or_else(|| kinds.iter().find(named))?;
        let scope = if found.namespaced {
            Scope::Namespaced
        } else {
            Scope::Cluster
        };
        Some((found.gvk.clone(), scope))
    }

    /// The object `group`/`kind` `name` of `namespace` (the panel's own namespace when `None`) as
    /// a reference to open.
    pub(super) fn target(
        &self,
        from: &ResourceRef,
        group: &str,
        kind: &str,
        namespace: Option<&str>,
        name: &str,
    ) -> Option<ResourceRef> {
        let (gvk, scope) = self.resolve(group, kind)?;
        Some(match scope {
            Scope::Cluster => ResourceRef::cluster_scoped(from.cluster.clone(), gvk, name),
            Scope::Namespaced => {
                let namespace = namespace.or(from.namespace())?;
                ResourceRef::namespaced(from.cluster.clone(), gvk, namespace, name)
            }
        })
    }
}
