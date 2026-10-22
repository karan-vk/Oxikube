//! [`KindScopes`]: which manifest kinds the cluster serves, and whether they are namespaced.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::kinds::ResourceKind;

/// The served version of a kind and its scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedKind {
    /// The version to watch: the manifest's own when the cluster serves it, else the preferred.
    pub gvk: Gvk,
    /// Whether objects of the kind live in a namespace.
    pub scope: Scope,
}

/// `(version, namespaced, preferred)` per served version.
type Versions = Vec<(Arc<str>, bool, bool)>;

/// What discovery (E04) says about the kinds a manifest may use. A kind it does not list (a CRD
/// that is not installed) is unknown: there is no hard-coded table of namespaced kinds.
#[derive(Debug, Default)]
pub struct KindScopes {
    kinds: HashMap<(Arc<str>, Arc<str>), Versions>,
}

impl KindScopes {
    /// The kinds of a discovery answer.
    pub fn from_kinds<'a>(kinds: impl IntoIterator<Item = &'a ResourceKind>) -> Self {
        let mut scopes = Self::default();
        for kind in kinds {
            scopes
                .kinds
                .entry((kind.gvk.group.clone(), kind.gvk.kind.clone()))
                .or_default()
                .push((kind.gvk.version.clone(), kind.namespaced, kind.preferred));
        }
        scopes
    }

    /// The served kind for a manifest's `apiVersion` + `kind`, or `None` when the cluster does
    /// not serve the kind at any version.
    pub fn resolve(&self, group: &str, version: &str, kind: &str) -> Option<ServedKind> {
        let versions = self.kinds.get(&(Arc::from(group), Arc::from(kind)))?;
        let (v, namespaced, _) = versions
            .iter()
            .find(|(v, ..)| &**v == version)
            .or_else(|| versions.iter().find(|(.., preferred)| *preferred))
            .or_else(|| versions.first())?;
        Some(ServedKind {
            gvk: Gvk::new(group, v.clone(), kind),
            scope: Scope::from_namespaced(*namespaced),
        })
    }

    /// Whether discovery listed nothing (not loaded yet).
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}
