//! [`IngressClassSummary`]: an IngressClass and whether it is the default.

use std::sync::Arc;

use super::super::{ViewError, check_kind, str_of, sub};
use crate::resource::Resource;

/// The annotation that makes an IngressClass the default one: an Ingress without a class gets
/// it. Its value is `"true"`.
pub const DEFAULT_CLASS_ANNOTATION: &str = "ingressclass.kubernetes.io/is-default-class";

/// `spec.parameters`: the object holding the controller's configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressClassParameters {
    /// API group of the parameters object, `None` for the core group.
    pub api_group: Option<String>,
    /// Kind of the parameters object.
    pub kind: String,
    /// Name of the parameters object.
    pub name: String,
    /// `Cluster` or `Namespace`; `Cluster` when the field is unset (the API default).
    pub scope: String,
    /// The namespace, for a `Namespace` scope.
    pub namespace: Option<String>,
}

/// An IngressClass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressClassSummary {
    /// `spec.controller`, for example `k8s.io/ingress-nginx`.
    pub controller: Option<Arc<str>>,
    /// `spec.parameters`.
    pub parameters: Option<IngressClassParameters>,
    /// Whether the class carries [`DEFAULT_CLASS_ANNOTATION`] set to `"true"`.
    pub is_default: bool,
}

impl IngressClassSummary {
    /// The summary of a `networking.k8s.io` IngressClass.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "IngressClass",
            &[("networking.k8s.io", "IngressClass")],
        )?;
        let spec = sub(res.json(), "spec");
        let parameters = spec
            .get("parameters")
            .filter(|p| p.is_object())
            .and_then(|p| {
                Some(IngressClassParameters {
                    api_group: str_of(p, "apiGroup").map(str::to_owned),
                    kind: str_of(p, "kind")?.to_owned(),
                    name: str_of(p, "name")?.to_owned(),
                    scope: str_of(p, "scope").unwrap_or("Cluster").to_owned(),
                    namespace: str_of(p, "namespace").map(str::to_owned),
                })
            });
        Ok(Self {
            controller: str_of(spec, "controller").map(Arc::from),
            parameters,
            is_default: is_default_class(res),
        })
    }
}

/// Whether `res` carries [`DEFAULT_CLASS_ANNOTATION`] set to `"true"`.
pub fn is_default_class(res: &Resource) -> bool {
    res.meta
        .annotations
        .get(DEFAULT_CLASS_ANNOTATION)
        .is_some_and(|value| &**value == "true")
}
