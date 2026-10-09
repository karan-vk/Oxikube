//! [`NodeSummary`].

use std::sync::Arc;

use jiff::Timestamp;

use super::{
    Condition, ConditionStatus, ViewError, arc_of, arr_of, bool_of, check_kind, str_of, sub,
};
use crate::age::Age;
use crate::quantity::Quantity;
use crate::resource::Resource;

/// Label prefix whose suffix names a role: `node-role.kubernetes.io/control-plane`.
const ROLE_PREFIX: &str = "node-role.kubernetes.io/";
/// Legacy label whose value names a role: `kubernetes.io/role=master`.
const ROLE_LABEL: &str = "kubernetes.io/role";

/// One row of a node table: the `kubectl get nodes -o wide` columns, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// Roles from `node-role.kubernetes.io/<role>` labels and the legacy `kubernetes.io/role`
    /// label, sorted and deduplicated. Empty when the node has none.
    pub roles: Vec<Arc<str>>,
    /// The `STATUS` column: `Ready` or `NotReady` (or `Unknown` without a `Ready` condition),
    /// plus `,SchedulingDisabled` when cordoned.
    pub status: Arc<str>,
    /// `status.conditions`, in API order.
    pub conditions: Vec<Condition>,
    /// `false` when `spec.unschedulable` is set (the node is cordoned).
    pub schedulable: bool,
    /// `status.nodeInfo.kubeletVersion`, the `VERSION` column.
    pub kubelet_version: Option<Arc<str>>,
    /// `status.nodeInfo.osImage`.
    pub os_image: Option<Arc<str>>,
    /// `status.nodeInfo.kernelVersion`.
    pub kernel_version: Option<Arc<str>>,
    /// `status.nodeInfo.containerRuntimeVersion`.
    pub container_runtime_version: Option<Arc<str>>,
    /// `status.nodeInfo.architecture`.
    pub architecture: Option<Arc<str>>,
    /// `status.nodeInfo.operatingSystem`.
    pub operating_system: Option<Arc<str>>,
    /// First `InternalIP` of `status.addresses`.
    pub internal_ip: Option<Arc<str>>,
    /// First `ExternalIP` of `status.addresses`.
    pub external_ip: Option<Arc<str>>,
    /// `status.allocatable.cpu`, when present and parseable.
    pub allocatable_cpu: Option<Quantity>,
    /// `status.allocatable.memory`, when present and parseable.
    pub allocatable_memory: Option<Quantity>,
    /// `status.allocatable.pods`, when present and parseable.
    pub allocatable_pods: Option<Quantity>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl NodeSummary {
    /// Build the summary of a Node.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a core `Node`. Missing or malformed fields
    /// never fail; they fall back to defaults.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Node", &[("", "Node")])?;
        let spec = sub(res.json(), "spec");
        let status = sub(res.json(), "status");
        let info = sub(status, "nodeInfo");
        let allocatable = sub(status, "allocatable");

        let conditions: Vec<Condition> = arr_of(status, "conditions")
            .iter()
            .filter_map(Condition::from_json)
            .collect();
        let schedulable = !bool_of(spec, "unschedulable");

        // The last `Ready` entry wins, as in the printer's condition map.
        let ready = conditions.iter().rev().find(|c| &*c.kind == "Ready");
        let readiness = match ready.map(|c| c.status) {
            Some(ConditionStatus::True) => "Ready",
            Some(_) => "NotReady",
            None => "Unknown",
        };
        let status_text: Arc<str> = if schedulable {
            Arc::from(readiness)
        } else {
            Arc::from(format!("{readiness},SchedulingDisabled"))
        };

        let address = |kind: &str| {
            arr_of(status, "addresses")
                .iter()
                .find(|&a| str_of(a, "type") == Some(kind))
                .and_then(|a| arc_of(a, "address"))
        };
        let quantity = |key: &str| str_of(allocatable, key).and_then(|q| Quantity::parse(q).ok());

        Ok(Self {
            name: res.meta.name.clone(),
            roles: roles(res),
            status: status_text,
            conditions,
            schedulable,
            kubelet_version: arc_of(info, "kubeletVersion"),
            os_image: arc_of(info, "osImage"),
            kernel_version: arc_of(info, "kernelVersion"),
            container_runtime_version: arc_of(info, "containerRuntimeVersion"),
            architecture: arc_of(info, "architecture"),
            operating_system: arc_of(info, "operatingSystem"),
            internal_ip: address("InternalIP"),
            external_ip: address("ExternalIP"),
            allocatable_cpu: quantity("cpu"),
            allocatable_memory: quantity("memory"),
            allocatable_pods: quantity("pods"),
            created: res.meta.creation,
        })
    }

    /// Whether the last `Ready` condition is `True`.
    pub fn is_ready(&self) -> bool {
        self.conditions
            .iter()
            .rev()
            .find(|c| &*c.kind == "Ready")
            .is_some_and(Condition::is_true)
    }

    /// Conditions other than `Ready` that are `True`: `MemoryPressure`, `DiskPressure`,
    /// `PIDPressure`, `NetworkUnavailable` and any custom problem condition.
    pub fn problems(&self) -> impl Iterator<Item = &Condition> {
        self.conditions
            .iter()
            .filter(|c| &*c.kind != "Ready" && c.is_true())
    }

    /// The `ROLES` column: comma-joined roles, or `<none>`.
    pub fn roles_display(&self) -> String {
        if self.roles.is_empty() {
            "<none>".to_owned()
        } else {
            self.roles.join(",")
        }
    }

    /// Age at `now`, if the creation time is known.
    pub fn age(&self, now: Timestamp) -> Option<Age> {
        self.created.map(|c| Age::between(c, now))
    }
}

/// Roles from the node's labels. Labels are a `BTreeMap`, but the two label forms interleave,
/// so the result is sorted and deduplicated explicitly.
fn roles(res: &Resource) -> Vec<Arc<str>> {
    let mut roles: Vec<Arc<str>> = Vec::new();
    for (key, value) in &res.meta.labels {
        if let Some(role) = key.strip_prefix(ROLE_PREFIX) {
            if !role.is_empty() {
                roles.push(Arc::from(role));
            }
        } else if &**key == ROLE_LABEL && !value.is_empty() {
            roles.push(value.clone());
        }
    }
    roles.sort_unstable();
    roles.dedup();
    roles
}
