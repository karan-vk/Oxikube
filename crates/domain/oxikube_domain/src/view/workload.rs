//! [`WorkloadSummary`] for Deployments, StatefulSets, DaemonSets and ReplicaSets.

use std::sync::Arc;

use jiff::Timestamp;

use super::{ViewError, bool_of, check_kind, count_of, opt_count, sub};
use crate::age::Age;
use crate::resource::Resource;

/// The pod controllers a [`WorkloadSummary`] can describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadKind {
    /// `apps/v1` Deployment.
    Deployment,
    /// `apps/v1` StatefulSet.
    StatefulSet,
    /// `apps/v1` DaemonSet.
    DaemonSet,
    /// `apps/v1` ReplicaSet.
    ReplicaSet,
}

impl WorkloadKind {
    /// The Kubernetes kind name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
            Self::ReplicaSet => "ReplicaSet",
        }
    }
}

/// Replica counts of a pod controller, in one shape for all four kinds.
///
/// | field | Deployment | StatefulSet | DaemonSet | ReplicaSet |
/// |---|---|---|---|---|
/// | `desired` | `spec.replicas` | `spec.replicas` | `status.desiredNumberScheduled` | `spec.replicas` |
/// | `current` | `status.replicas` | `status.replicas` | `status.currentNumberScheduled` | `status.replicas` |
/// | `ready` | `status.readyReplicas` | `status.readyReplicas` | `status.numberReady` | `status.readyReplicas` |
/// | `updated` | `status.updatedReplicas` | `status.updatedReplicas` | `status.updatedNumberScheduled` | same as `current` |
/// | `available` | `status.availableReplicas` | `status.availableReplicas` | `status.numberAvailable` | `status.availableReplicas` |
///
/// An absent `spec.replicas` means 1, the API server default. Every other absent count is 0. A
/// ReplicaSet runs a single pod template, so all of its pods count as updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadSummary {
    /// Which controller this is.
    pub kind: WorkloadKind,
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// Pods the controller wants.
    pub desired: u32,
    /// Pods that exist.
    pub current: u32,
    /// Pods that are ready.
    pub ready: u32,
    /// Pods running the latest template.
    pub updated: u32,
    /// Pods ready for at least `minReadySeconds`.
    pub available: u32,
    /// Deployment `spec.paused`; `false` for other kinds.
    pub paused: bool,
    /// StatefulSet `spec.updateStrategy.rollingUpdate.partition`; `None` for other kinds or
    /// when unset.
    pub partition: Option<u32>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl WorkloadSummary {
    /// Build the summary of an `apps` Deployment, StatefulSet, DaemonSet or ReplicaSet.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind. Missing or malformed fields never fail;
    /// they fall back to defaults.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        const EXPECTED: &str = "Deployment, StatefulSet, DaemonSet or ReplicaSet";
        check_kind(
            res,
            EXPECTED,
            &[
                ("apps", "Deployment"),
                ("apps", "StatefulSet"),
                ("apps", "DaemonSet"),
                ("apps", "ReplicaSet"),
            ],
        )?;
        let spec = sub(res.json(), "spec");
        let status = sub(res.json(), "status");
        let replicas = opt_count(spec, "replicas").unwrap_or(1);

        let (kind, desired, current, ready, updated, available) = match &*res.kind.kind {
            "Deployment" => (
                WorkloadKind::Deployment,
                replicas,
                count_of(status, "replicas"),
                count_of(status, "readyReplicas"),
                count_of(status, "updatedReplicas"),
                count_of(status, "availableReplicas"),
            ),
            "StatefulSet" => (
                WorkloadKind::StatefulSet,
                replicas,
                count_of(status, "replicas"),
                count_of(status, "readyReplicas"),
                count_of(status, "updatedReplicas"),
                count_of(status, "availableReplicas"),
            ),
            "DaemonSet" => (
                WorkloadKind::DaemonSet,
                count_of(status, "desiredNumberScheduled"),
                count_of(status, "currentNumberScheduled"),
                count_of(status, "numberReady"),
                count_of(status, "updatedNumberScheduled"),
                count_of(status, "numberAvailable"),
            ),
            _ => {
                let current = count_of(status, "replicas");
                (
                    WorkloadKind::ReplicaSet,
                    replicas,
                    current,
                    count_of(status, "readyReplicas"),
                    current,
                    count_of(status, "availableReplicas"),
                )
            }
        };

        let partition = match kind {
            WorkloadKind::StatefulSet => opt_count(
                sub(sub(spec, "updateStrategy"), "rollingUpdate"),
                "partition",
            ),
            _ => None,
        };

        Ok(Self {
            kind,
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            desired,
            current,
            ready,
            updated,
            available,
            paused: kind == WorkloadKind::Deployment && bool_of(spec, "paused"),
            partition,
            created: res.meta.creation,
        })
    }

    /// Desired pods that are not available (never negative).
    pub fn unavailable(&self) -> u32 {
        self.desired.saturating_sub(self.available)
    }

    /// Whether every desired pod is updated, ready and available.
    pub fn is_settled(&self) -> bool {
        self.updated >= self.desired && self.ready >= self.desired && self.available >= self.desired
    }

    /// The `READY` column of Deployments and StatefulSets, `ready/desired`.
    pub fn ready_display(&self) -> String {
        format!("{}/{}", self.ready, self.desired)
    }

    /// Age at `now`, if the creation time is known.
    pub fn age(&self, now: Timestamp) -> Option<Age> {
        self.created.map(|c| Age::between(c, now))
    }
}
