//! The seam between the reconnect logic and the API server.
//!
//! [`LogSource`] is what the follower needs from Kubernetes: open a log stream, read the
//! pod's state, watch a set of pods. [`KubeSource`](super::kube_source::KubeSource) is the
//! kube-rs implementation; the unit tests script a fake, so reconnect, overlap and dedup are
//! tested without a cluster and without wall-clock time. The types here are plain data:
//! no kube or k8s-openapi type crosses the seam.

use std::pin::Pin;

use async_trait::async_trait;
use futures::io::AsyncBufRead;
use futures::stream::BoxStream;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{LogOptions, LogSince};

/// A server log response as a buffered byte reader.
pub(crate) type Reader = Pin<Box<dyn AsyncBufRead + Send>>;

/// One `pods/log` request. Timestamps are always requested: dedup keys on them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenRequest {
    pub(crate) container: String,
    pub(crate) follow: bool,
    pub(crate) previous: bool,
    pub(crate) since: Option<LogSince>,
    pub(crate) tail_lines: Option<i64>,
    pub(crate) limit_bytes: Option<i64>,
}

impl OpenRequest {
    /// The first open of `container` for `options`.
    pub(crate) fn first(container: &str, options: &LogOptions) -> Self {
        Self {
            container: container.to_owned(),
            follow: options.follow && !options.previous,
            previous: options.previous,
            since: options.since,
            tail_lines: options.tail_lines,
            limit_bytes: options.limit_bytes,
        }
    }

    /// A reopen from `since`: the live log (`previous` false, followed) or the previous
    /// instance's (read to its end).
    pub(crate) fn resume(container: &str, previous: bool, since: LogSince) -> Self {
        Self {
            container: container.to_owned(),
            follow: !previous,
            previous,
            since: Some(since),
            tail_lines: None,
            limit_bytes: None,
        }
    }
}

/// `spec.restartPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

/// `status.phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

/// Where a container is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerState {
    /// Not started yet, or between restarts (CrashLoopBackOff). No log to read.
    Waiting,
    Running,
    Terminated {
        exit_code: i32,
    },
    /// The status has no state yet.
    Unknown,
}

/// What a container's log reader needs to know about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerInfo {
    pub(crate) name: String,
    pub(crate) restart_count: i32,
    pub(crate) state: ContainerState,
    /// Declared in `spec.initContainers`.
    pub(crate) init: bool,
}

impl ContainerInfo {
    /// Whether the container has run, so there is a log to read.
    pub(crate) fn has_started(&self) -> bool {
        matches!(
            self.state,
            ContainerState::Running | ContainerState::Terminated { .. }
        )
    }
}

/// The parts of a pod the log reader looks at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PodInfo {
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) uid: String,
    pub(crate) deleting: bool,
    pub(crate) phase: PodPhase,
    pub(crate) restart_policy: RestartPolicy,
    /// `kubectl.kubernetes.io/default-container`, which `pods/log` honours.
    pub(crate) default_container: Option<String>,
    /// Init containers first, then regular, then ephemeral, as `kubectl logs --all-containers`.
    pub(crate) containers: Vec<ContainerInfo>,
}

impl PodInfo {
    /// The status of `name`, if the pod has such a container.
    pub(crate) fn container(&self, name: &str) -> Option<&ContainerInfo> {
        self.containers.iter().find(|c| c.name == name)
    }

    /// Whether `container` will run again after its current instance ends.
    pub(crate) fn will_restart(&self, container: &ContainerInfo) -> bool {
        if self.deleting || matches!(self.phase, PodPhase::Succeeded | PodPhase::Failed) {
            return false;
        }
        if container.init {
            // An init container is retried on failure unless the pod never restarts.
            return !matches!(container.state, ContainerState::Terminated { exit_code: 0 })
                && self.restart_policy != RestartPolicy::Never;
        }
        match self.restart_policy {
            RestartPolicy::Always => true,
            RestartPolicy::OnFailure => {
                !matches!(container.state, ContainerState::Terminated { exit_code: 0 })
            }
            RestartPolicy::Never => false,
        }
    }

    /// The container a request without a name reads, resolved as the API server does:
    /// the default-container annotation, else the only regular container.
    pub(crate) fn default_container_name(&self) -> Option<&str> {
        if let Some(name) = self.default_container.as_deref() {
            if self.container(name).is_some() {
                return Some(name);
            }
        }
        let mut regular = self.containers.iter().filter(|c| !c.init);
        match (regular.next(), regular.next()) {
            (Some(only), None) => Some(only.name.as_str()),
            _ => None,
        }
    }
}

/// A change in the watched pod set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PodEvent {
    /// A pod that exists (initial listing) or was added or changed (`initial` false).
    Pod { pod: PodInfo, initial: bool },
    /// The initial listing is complete.
    InitDone,
}

/// What the log reader needs from the cluster.
#[async_trait]
pub(crate) trait LogSource: Send + Sync + 'static {
    /// Opens a log response. Errors are already mapped to `OxiError`.
    async fn open(&self, namespace: &str, pod: &str, request: &OpenRequest) -> OxiResult<Reader>;

    /// The pod, or `None` when it does not exist.
    async fn pod(&self, namespace: &str, name: &str) -> OxiResult<Option<PodInfo>>;

    /// Watches pods matching a label selector (`namespace` `None` is all namespaces). The
    /// stream retries transient failures itself; an `Err` item is one that retrying will not fix.
    fn watch_pods(
        &self,
        namespace: Option<&str>,
        selector: &str,
    ) -> BoxStream<'static, OxiResult<PodEvent>>;
}

/// The pod, or the `NotFound` a port call reports for one that does not exist.
pub(crate) async fn require_pod(
    source: &dyn LogSource,
    namespace: &str,
    name: &str,
) -> OxiResult<PodInfo> {
    source
        .pod(namespace, name)
        .await?
        .ok_or_else(|| OxiError::not_found(format!("pod {namespace}/{name} does not exist")))
}
