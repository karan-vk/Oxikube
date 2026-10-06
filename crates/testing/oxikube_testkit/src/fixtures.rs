//! Realistic JSON manifests under `crates/testing/oxikube_testkit/fixtures/`, loaded as
//! domain [`Resource`]s.
//!
//! The files are embedded with `include_str!` and parsed only when asked for, so a test
//! that needs one pod does not parse the other manifests. Load by path
//! (`fixtures::load("pods/crashloop.json")`) or with the named function
//! (`fixtures::pod_crashloop()`). Variations belong in the builders
//! ([`crate::builders`]), which share the fixtures' defaults (namespace `demo`, node
//! `worker-1`, creation `2026-01-01T00:00:00Z`).
//!
//! The kind manifests under `fixtures/cluster/` are applied to a real cluster by
//! `cargo xtask kind-up` and are not part of this set. Secret and Helm fixtures hold
//! obvious dummy data only.

use std::fmt;

use oxikube_domain::{Resource, ResourceError};
use serde_json::Value;

/// Why a fixture could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureError {
    /// No fixture has this path (see [`ALL`]).
    Unknown(String),
    /// The file is not valid JSON.
    Json {
        /// Fixture path.
        path: String,
        /// Parser message.
        message: String,
    },
    /// The JSON is not a valid `Resource`.
    Resource {
        /// Fixture path.
        path: String,
        /// Why `Resource::from_json` refused it.
        error: ResourceError,
    },
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(path) => write!(f, "unknown fixture {path:?}"),
            Self::Json { path, message } => write!(f, "fixture {path}: invalid JSON: {message}"),
            Self::Resource { path, error } => write!(f, "fixture {path}: {error}"),
        }
    }
}

impl std::error::Error for FixtureError {}

macro_rules! fixtures {
    ($($(#[$doc:meta])* $fn_name:ident => $path:literal,)*) => {
        /// Every fixture path, relative to the `fixtures/` directory.
        pub const ALL: &[&str] = &[$($path),*];

        /// The raw text of the fixture at `path`, or `None` when there is none.
        pub fn raw(path: &str) -> Option<&'static str> {
            match path {
                $($path => Some(include_str!(concat!("../fixtures/", $path))),)*
                _ => None,
            }
        }

        $(
            $(#[$doc])*
            ///
            #[doc = concat!("Loads `fixtures/", $path, "`.")]
            pub fn $fn_name() -> Resource {
                load($path)
            }
        )*
    };
}

fixtures! {
    /// Pod `web-pending`: unschedulable. Status `Pending`.
    pod_pending => "pods/pending.json",
    /// Pod `web-creating`: container waiting `ContainerCreating`.
    pod_container_creating => "pods/container-creating.json",
    /// Pod `web-running`: running and ready. Status `Running`.
    pod_running => "pods/running.json",
    /// Pod `web-restarted`: running with 3 restarts.
    pod_running_restarted => "pods/running-restarted.json",
    /// Pod `web-succeeded`: phase `Succeeded`. Status `Completed`.
    pod_succeeded => "pods/succeeded.json",
    /// Pod `web-failed`: container exited 1. Status `Error`.
    pod_failed => "pods/failed.json",
    /// Pod `web-evicted`: phase `Failed`, reason `Evicted`.
    pod_evicted => "pods/evicted.json",
    /// Pod `web-crashloop`: `CrashLoopBackOff` with 5 restarts.
    pod_crashloop => "pods/crashloop.json",
    /// Pod `web-imagepull`: `ImagePullBackOff`.
    pod_image_pull_backoff => "pods/image-pull-backoff.json",
    /// Pod `web-errimagepull`: `ErrImagePull`.
    pod_err_image_pull => "pods/err-image-pull.json",
    /// Pod `web-oomkilled`: container terminated `OOMKilled`.
    pod_oom_killed => "pods/oom-killed.json",
    /// Pod `web-init`: one of two init containers done. Status `Init:1/2`.
    pod_init => "pods/init.json",
    /// Pod `web-terminating`: running with a `deletionTimestamp`. Status `Terminating`.
    pod_terminating => "pods/terminating.json",
    /// Pod `web-nodelost`: `status.reason` `NodeLost`.
    pod_node_lost => "pods/node-lost.json",
    /// Pod `web-sidecar`: native sidecar plus app container. Status `Running`, 2/2.
    pod_sidecar => "pods/sidecar.json",
    /// Deployment `web`: 3/3 ready.
    deployment_ready => "workloads/deployment.json",
    /// Deployment `web` mid-rollout: 3 desired, 2 ready and available.
    deployment_progressing => "workloads/deployment-progressing.json",
    /// ReplicaSet `web-5d8c7b9f4` owned by Deployment `web`.
    replicaset => "workloads/replicaset.json",
    /// StatefulSet `db`: 3 replicas, partition 1.
    statefulset => "workloads/statefulset.json",
    /// DaemonSet `agent`: 2 desired, 1 ready.
    daemonset => "workloads/daemonset.json",
    /// Job `migrate`: 3/3 completions.
    job_complete => "workloads/job-complete.json",
    /// Job `migrate-failed`: backoff limit exceeded.
    job_failed => "workloads/job-failed.json",
    /// CronJob `nightly-backup` with one active job.
    cronjob => "workloads/cronjob.json",
    /// Node `worker-1`: ready worker.
    node_ready => "nodes/ready.json",
    /// Node `oxikube-control-plane`: ready, role `control-plane`.
    node_control_plane => "nodes/control-plane.json",
    /// Node `worker-2`: kubelet stopped posting status. Status `NotReady`.
    node_not_ready => "nodes/not-ready.json",
    /// Node `worker-3`: cordoned. Status `Ready,SchedulingDisabled`.
    node_cordoned => "nodes/cordoned.json",
    /// Node `worker-4`: ready with `MemoryPressure`.
    node_memory_pressure => "nodes/memory-pressure.json",
    /// CustomResourceDefinition `widgets.test.oxikube.dev` (printer columns, as in the kind
    /// fixtures).
    widget_crd => "crds/widget-crd.json",
    /// Custom resource Widget `widget-large`.
    widget => "crds/widget.json",
    /// core/v1 Event: `Warning` `BackOff` on pod `web-crashloop`, count 42.
    event_core_warning => "events/core-warning.json",
    /// core/v1 Event: `Normal` `Scheduled` on pod `web-running`.
    event_core_normal => "events/core-normal.json",
    /// events.k8s.io/v1 Event: `ScalingReplicaSet` on Deployment `web`, with a series.
    event_events_v1 => "events/events-v1.json",
    /// Helm release Secret (`helm.sh/release.v1`) for release `web` revision 2:
    /// `data.release` is base64(base64(gzip(release JSON))), with dummy content.
    helm_release_secret => "helm/release-secret.json",
    /// Namespace `demo`.
    namespace => "core/namespace.json",
    /// Service `web` (ClusterIP).
    service => "core/service.json",
    /// PersistentVolumeClaim `data-db-0`: `Bound`, 10Gi, `ReadWriteOnce`, class `standard`.
    pvc => "core/pvc.json",
    /// Ingress `web` (class `nginx`, one host rule and one hostless rule, TLS, one address).
    ingress => "networking/ingress.json",
    /// `autoscaling/v2` HorizontalPodAutoscaler `web` targeting Deployment `web` (cpu 80%
    /// utilisation, memory 512Mi); only the cpu metric has reported (42%).
    hpa => "autoscaling/hpa.json",
    /// ConfigMap `web-config`.
    configmap => "core/configmap.json",
    /// Opaque Secret `web-credentials` with dummy values.
    secret => "core/secret.json",
}

/// The fixture at `path` as JSON.
///
/// # Panics
///
/// When there is no such fixture or it is not valid JSON.
pub fn json(path: &str) -> Value {
    match try_json(path) {
        Ok(value) => value,
        Err(e) => panic!("{e}"),
    }
}

/// The fixture at `path` as JSON, or why it could not be read.
pub fn try_json(path: &str) -> Result<Value, FixtureError> {
    let text = raw(path).ok_or_else(|| FixtureError::Unknown(path.to_owned()))?;
    serde_json::from_str(text).map_err(|e| FixtureError::Json {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

/// The fixture at `path` as a domain [`Resource`], or why it could not be loaded.
pub fn try_load(path: &str) -> Result<Resource, FixtureError> {
    Resource::from_json(try_json(path)?).map_err(|error| FixtureError::Resource {
        path: path.to_owned(),
        error,
    })
}

/// The fixture at `path` (for example `"pods/crashloop.json"`) as a domain [`Resource`].
///
/// # Panics
///
/// When there is no such fixture or it does not parse; fixtures are checked by this
/// crate's tests, so a panic here means a typo in the path.
pub fn load(path: &str) -> Resource {
    match try_load(path) {
        Ok(res) => res,
        Err(e) => panic!("{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_paths_are_reported() {
        assert_eq!(
            try_load("pods/nope.json"),
            Err(FixtureError::Unknown("pods/nope.json".into()))
        );
        assert!(raw("cluster/10-namespaces/oxikube-fixtures.yaml").is_none());
        assert!(
            FixtureError::Unknown("x".into())
                .to_string()
                .contains("unknown fixture")
        );
    }

    #[test]
    fn named_functions_load_their_file() {
        assert_eq!(pod_crashloop(), load("pods/crashloop.json"));
        assert_eq!(pod_crashloop().name(), "web-crashloop");
        assert_eq!(namespace().namespace(), None);
    }
}
