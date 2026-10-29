//! [`ManifestObjects`]: the objects one `resource::ApplyPlan` carries, and their risk.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{Gvk, Scope};
use crate::safety::{MutationFlags, MutationVerb, Risk, classify};

/// The objects of a multi-object apply (E20-S03), as JSON, in the order they were given.
///
/// They may be Secrets: the `Debug` form names how many there are and their kinds, never a
/// field, and nothing here is logged. Serialised as a plain array.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManifestObjects(pub Vec<Value>);

impl ManifestObjects {
    /// How many objects there are.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The objects.
    pub fn iter(&self) -> std::slice::Iter<'_, Value> {
        self.0.iter()
    }

    /// The risk of applying them all: the highest [`object_apply_risk`] of any object, so one
    /// cluster-scoped, RBAC or custom kind raises the whole batch (it is confirmed once, at that
    /// tier). [`Risk::Low`] for no objects.
    pub fn apply_risk(&self, force: bool) -> Risk {
        self.iter()
            .map(|object| object_apply_risk(object, force))
            .max()
            .unwrap_or(Risk::Low)
    }
}

impl fmt::Debug for ManifestObjects {
    /// Counts and kinds only: an object may be a Secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kinds: Vec<&str> = self
            .iter()
            .map(|object| object.get("kind").and_then(Value::as_str).unwrap_or("?"))
            .collect();
        f.debug_struct("ManifestObjects")
            .field("count", &self.len())
            .field("kinds", &kinds)
            .finish()
    }
}

impl From<Vec<Value>> for ManifestObjects {
    fn from(objects: Vec<Value>) -> Self {
        Self(objects)
    }
}

/// The risk of server-side applying `object` ([`classify`] with [`MutationVerb::Apply`]): its
/// kind's row, raised to at least [`Risk::Medium`] for a custom kind or a cluster-scoped object,
/// and for `force` (taking over other managers' fields).
///
/// The scope is read from the object itself: one that names no `metadata.namespace` counts as
/// cluster-scoped, which can only overstate the risk (a plan defaults the namespace of every
/// namespaced kind it knows before it is sent).
pub fn object_apply_risk(object: &Value, force: bool) -> Risk {
    let text = |pointer: &str| object.pointer(pointer).and_then(Value::as_str);
    let gvk = Gvk::from_api_version(
        text("/apiVersion").unwrap_or_default(),
        text("/kind").unwrap_or_default(),
    );
    let scope = match text("/metadata/namespace") {
        Some(namespace) if !namespace.is_empty() => Scope::Namespaced,
        _ => Scope::Cluster,
    };
    let flags = MutationFlags {
        force,
        ..MutationFlags::default()
    };
    classify(MutationVerb::Apply, scope, &gvk, flags)
}
