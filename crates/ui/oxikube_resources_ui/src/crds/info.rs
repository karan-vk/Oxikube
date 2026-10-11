//! [`CrdInfo`]: a CustomResourceDefinition read for browsing.

use oxikube_domain::ids::{Gvk, Scope};
use serde_json::Value;

/// The CRD kind itself: what the CRD list shows.
pub fn crd_gvk() -> Gvk {
    Gvk::new("apiextensions.k8s.io", "v1", "CustomResourceDefinition")
}

/// Whether `gvk` is the CustomResourceDefinition kind, whichever version of the API serves it.
pub fn is_crd_kind(gvk: &Gvk) -> bool {
    &*gvk.group == "apiextensions.k8s.io" && &*gvk.kind == "CustomResourceDefinition"
}

/// One entry of `spec.versions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrdVersion {
    /// The version name (`v1alpha1`).
    pub name: String,
    /// Whether the API server serves it.
    pub served: bool,
    /// Whether objects are stored at this version.
    pub storage: bool,
    /// Whether the CRD marks it deprecated.
    pub deprecated: bool,
    /// Whether it declares an `openAPIV3Schema`.
    pub has_schema: bool,
}

/// A CustomResourceDefinition as the browser needs it. Parsed from the object's JSON; a field
/// that is missing reads as empty, so a half-written CRD still lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrdInfo {
    /// The CRD's name (`widgets.example.com`).
    pub name: String,
    /// The API group of the resources it defines.
    pub group: String,
    /// The kind (`Widget`).
    pub kind: String,
    /// The plural resource name (`widgets`).
    pub plural: String,
    /// Namespaced or cluster-scoped.
    pub scope: Scope,
    /// `spec.names.shortNames`.
    pub short_names: Vec<String>,
    /// `spec.names.categories`.
    pub categories: Vec<String>,
    /// `spec.versions`, in the CRD's order.
    pub versions: Vec<CrdVersion>,
}

impl CrdInfo {
    /// Reads a CRD from its JSON; `None` when it is not one (no `spec.group` or `spec.names.kind`).
    pub fn parse(crd: &Value) -> Option<Self> {
        let spec = crd.get("spec")?;
        let names = spec.get("names")?;
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        let strings = |v: &Value, key: &str| -> Vec<String> {
            v.get(key)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let flag = |v: &Value, key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
        let versions = spec
            .get("versions")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|v| {
                        Some(CrdVersion {
                            name: text(v, "name")?,
                            served: flag(v, "served"),
                            storage: flag(v, "storage"),
                            deprecated: flag(v, "deprecated"),
                            has_schema: v.pointer("/schema/openAPIV3Schema").is_some(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            name: crd.pointer("/metadata/name")?.as_str()?.to_owned(),
            group: text(spec, "group")?,
            kind: text(names, "kind")?,
            plural: text(names, "plural")?,
            scope: match text(spec, "scope").as_deref() {
                Some("Cluster") => Scope::Cluster,
                _ => Scope::Namespaced,
            },
            short_names: strings(names, "shortNames"),
            categories: strings(names, "categories"),
            versions,
        })
    }

    /// The version a table opens: the storage version when it is served, else the newest served
    /// one ([`version_order`]). `None` when the CRD serves nothing.
    pub fn display_version(&self) -> Option<&CrdVersion> {
        self.versions
            .iter()
            .find(|v| v.storage && v.served)
            .or_else(|| {
                self.versions
                    .iter()
                    .filter(|v| v.served)
                    .min_by(|a, b| version_order(&a.name, &b.name))
            })
    }

    /// The type of the custom resources at `version`.
    pub fn gvk(&self, version: &str) -> Gvk {
        Gvk::new(self.group.as_str(), version, self.kind.as_str())
    }
}

/// Kubernetes' version priority (newest first), from the domain's CRD view-model.
pub use oxikube_domain::view::version_order;
