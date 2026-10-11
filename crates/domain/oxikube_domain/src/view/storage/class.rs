//! [`StorageClassSummary`] and the default-class rule.

use std::sync::Arc;

use jiff::Timestamp;

use super::access::ReclaimPolicy;
use super::{ViewError, arc_of, bool_of, check_kind, pairs_of, str_of, strings_of};
use crate::json::JsonRef;
use crate::resource::{ObjectMeta, Resource};

/// The annotation that marks the default StorageClass.
pub const DEFAULT_CLASS_ANNOTATION: &str = "storageclass.kubernetes.io/is-default-class";
/// The legacy (beta) spelling, still honoured by the API server and read for display.
pub const BETA_DEFAULT_CLASS_ANNOTATION: &str = "storageclass.beta.kubernetes.io/is-default-class";

/// Whether `meta` marks a default StorageClass: either annotation is `"true"`, as the API
/// server's `IsDefaultAnnotation` reads them.
pub fn is_default_class(meta: &ObjectMeta) -> bool {
    [DEFAULT_CLASS_ANNOTATION, BETA_DEFAULT_CLASS_ANNOTATION]
        .iter()
        .any(|key| meta.annotations.get(key).is_some_and(|v| &**v == "true"))
}

/// One StorageClass, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageClassSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `provisioner`.
    pub provisioner: Option<Arc<str>>,
    /// `reclaimPolicy`, `Delete` when the class names none (the API default).
    pub reclaim_policy: ReclaimPolicy,
    /// `volumeBindingMode`, `Immediate` when the class names none (the API default).
    pub binding_mode: Arc<str>,
    /// `allowVolumeExpansion`: claims of this class may be resized. `false` when absent.
    pub allow_expansion: bool,
    /// Either default-class annotation is `"true"`.
    pub is_default: bool,
    /// `parameters`, sorted by key.
    pub parameters: Vec<(Arc<str>, Arc<str>)>,
    /// `mountOptions`.
    pub mount_options: Vec<Arc<str>>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl StorageClassSummary {
    /// Builds the summary of a `storage.k8s.io` StorageClass.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a StorageClass.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "StorageClass", &[("storage.k8s.io", "StorageClass")])?;
        let json: JsonRef<'_> = res.json();
        Ok(Self {
            name: res.meta.name.clone(),
            provisioner: arc_of(json, "provisioner"),
            reclaim_policy: str_of(json, "reclaimPolicy")
                .map_or(ReclaimPolicy::Delete, ReclaimPolicy::parse),
            binding_mode: arc_of(json, "volumeBindingMode")
                .unwrap_or_else(|| Arc::from("Immediate")),
            allow_expansion: bool_of(json, "allowVolumeExpansion"),
            is_default: is_default_class(&res.meta),
            parameters: pairs_of(json, "parameters"),
            mount_options: strings_of(json, "mountOptions"),
            created: res.meta.creation,
        })
    }
}
