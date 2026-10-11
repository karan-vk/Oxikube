//! [`GatewayClassSummary`].

use std::sync::Arc;

use super::super::super::{Condition, ConditionStatus, ViewError, arc_of, check_kind, str_of, sub};
use super::{GATEWAY_API_GROUP, ObjectRef, conditions_of, status_of};
use crate::resource::Resource;

/// A GatewayClass: the controller that implements Gateways of the class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayClassSummary {
    /// `spec.controllerName`, for example `example.com/gateway-controller`.
    pub controller: Option<Arc<str>>,
    /// `spec.description`.
    pub description: Option<Arc<str>>,
    /// `spec.parametersRef`: the object configuring the controller for this class.
    pub parameters: Option<ObjectRef>,
    /// `status.conditions`, with their reasons.
    pub conditions: Vec<Condition>,
}

impl GatewayClassSummary {
    /// The summary of a Gateway API GatewayClass.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "GatewayClass", &[(GATEWAY_API_GROUP, "GatewayClass")])?;
        let spec = sub(res.json(), "spec");
        let parameters = spec.get("parametersRef").and_then(|p| {
            Some(ObjectRef::new(
                super::group_of(p, ""),
                str_of(p, "kind")?,
                str_of(p, "namespace"),
                str_of(p, "name")?,
            ))
        });
        Ok(Self {
            controller: arc_of(spec, "controllerName"),
            description: arc_of(spec, "description"),
            parameters,
            conditions: conditions_of(sub(res.json(), "status")),
        })
    }

    /// The `Accepted` condition's status: whether the controller accepted the class.
    pub fn accepted(&self) -> ConditionStatus {
        status_of(&self.conditions, "Accepted")
    }
}
