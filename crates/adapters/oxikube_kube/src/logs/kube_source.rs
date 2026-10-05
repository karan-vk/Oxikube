//! [`LogSource`] on kube-rs: `Api<Pod>::log_stream`, `get_opt` and `watcher`.

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::{StreamExt, future};
use k8s_openapi::api::core::v1::{ContainerStatus, Pod};
use kube::api::LogParams;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Event};
use kube::{Api, Client};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::LogSince;

use super::source::{
    ContainerInfo, ContainerState, LogSource, OpenRequest, PodEvent, PodInfo, PodPhase, Reader,
    RestartPolicy,
};
use crate::auth::classify;

/// The annotation `pods/log` reads to pick a container when none is named.
const DEFAULT_CONTAINER_ANNOTATION: &str = "kubectl.kubernetes.io/default-container";

/// Log reads through one connected cluster's client.
pub(crate) struct KubeSource {
    client: Client,
}

impl KubeSource {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    fn pods(&self, namespace: &str) -> Api<Pod> {
        Api::namespaced(self.client.clone(), namespace)
    }
}

#[async_trait]
impl LogSource for KubeSource {
    async fn open(&self, namespace: &str, pod: &str, request: &OpenRequest) -> OxiResult<Reader> {
        let mut params = LogParams {
            container: Some(request.container.clone()),
            follow: request.follow,
            previous: request.previous,
            tail_lines: request.tail_lines,
            limit_bytes: request.limit_bytes,
            timestamps: true,
            ..LogParams::default()
        };
        match request.since {
            Some(LogSince::Seconds(secs)) => params.since_seconds = Some(secs),
            Some(LogSince::Time(at)) => params.since_time = Some(at),
            None => {}
        }
        let reader = self
            .pods(namespace)
            .log_stream(pod, &params)
            .await
            .map_err(|e| classify(&e))?;
        Ok(Box::pin(reader))
    }

    async fn pod(&self, namespace: &str, name: &str) -> OxiResult<Option<PodInfo>> {
        let pod = self
            .pods(namespace)
            .get_opt(name)
            .await
            .map_err(|e| classify(&e))?;
        Ok(pod.map(|pod| pod_info(&pod)))
    }

    fn watch_pods(
        &self,
        namespace: Option<&str>,
        selector: &str,
    ) -> BoxStream<'static, OxiResult<PodEvent>> {
        let api: Api<Pod> = match namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };
        watcher::watcher(api, watcher::Config::default().labels(selector))
            .default_backoff()
            .filter_map(|event| {
                future::ready(match event {
                    Ok(Event::InitApply(pod)) => Some(Ok(PodEvent::Pod {
                        pod: pod_info(&pod),
                        initial: true,
                    })),
                    Ok(Event::Apply(pod)) => Some(Ok(PodEvent::Pod {
                        pod: pod_info(&pod),
                        initial: false,
                    })),
                    Ok(Event::InitDone) => Some(Ok(PodEvent::InitDone)),
                    Ok(Event::Init | Event::Delete(_)) => None,
                    Err(err) => watch_failure(&err).map(Err),
                })
            })
            .boxed()
    }
}

/// A watch failure worth reporting. Network blips and expired watches retry (the backoff
/// wrapper re-polls the watcher), so they are only logged; permission and API-support
/// failures will not heal, so they end the fan-in.
fn watch_failure(err: &watcher::Error) -> Option<OxiError> {
    let mapped = match err {
        watcher::Error::InitialListFailed(e)
        | watcher::Error::WatchStartFailed(e)
        | watcher::Error::WatchFailed(e) => classify(e),
        watcher::Error::WatchError(status) => {
            classify(&kube::Error::Api(status.as_ref().clone().into()))
        }
        watcher::Error::NoResourceVersion => {
            OxiError::unsupported("the cluster did not return a resourceVersion for pods")
        }
    };
    if matches!(
        mapped.kind(),
        ErrorKind::Auth | ErrorKind::Forbidden | ErrorKind::Unsupported
    ) && !mapped.is_retryable()
    {
        return Some(mapped);
    }
    tracing::debug!(kind = ?mapped.kind(), "pod watch for log fan-in failed; retrying");
    None
}

/// The log reader's view of a pod.
pub(crate) fn pod_info(pod: &Pod) -> PodInfo {
    let spec = pod.spec.as_ref();
    let status = pod.status.as_ref();
    let mut containers = Vec::new();
    containers.extend(container_infos(
        spec.and_then(|s| s.init_containers.as_deref())
            .unwrap_or_default()
            .iter()
            .map(|c| c.name.as_str()),
        status.and_then(|s| s.init_container_statuses.as_deref()),
        true,
    ));
    containers.extend(container_infos(
        spec.map(|s| s.containers.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|c| c.name.as_str()),
        status.and_then(|s| s.container_statuses.as_deref()),
        false,
    ));
    containers.extend(container_infos(
        spec.and_then(|s| s.ephemeral_containers.as_deref())
            .unwrap_or_default()
            .iter()
            .map(|c| c.name.as_str()),
        status.and_then(|s| s.ephemeral_container_statuses.as_deref()),
        false,
    ));
    PodInfo {
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        name: pod.metadata.name.clone().unwrap_or_default(),
        uid: pod.metadata.uid.clone().unwrap_or_default(),
        deleting: pod.metadata.deletion_timestamp.is_some(),
        phase: match status.and_then(|s| s.phase.as_deref()) {
            Some("Pending") => PodPhase::Pending,
            Some("Running") => PodPhase::Running,
            Some("Succeeded") => PodPhase::Succeeded,
            Some("Failed") => PodPhase::Failed,
            _ => PodPhase::Unknown,
        },
        restart_policy: match spec.and_then(|s| s.restart_policy.as_deref()) {
            Some("Never") => RestartPolicy::Never,
            Some("OnFailure") => RestartPolicy::OnFailure,
            _ => RestartPolicy::Always,
        },
        default_container: pod
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(DEFAULT_CONTAINER_ANNOTATION))
            .cloned(),
        containers,
    }
}

/// The declared containers of one group, each joined with its status (if it has one yet).
fn container_infos<'a>(
    declared: impl Iterator<Item = &'a str> + 'a,
    statuses: Option<&'a [ContainerStatus]>,
    init: bool,
) -> impl Iterator<Item = ContainerInfo> + 'a {
    let statuses = statuses.unwrap_or_default();
    declared.map(move |name| {
        let status = statuses.iter().find(|s| s.name == name);
        ContainerInfo {
            name: name.to_owned(),
            restart_count: status.map_or(0, |s| s.restart_count),
            state: status.map_or(ContainerState::Unknown, container_state),
            init,
        }
    })
}

fn container_state(status: &ContainerStatus) -> ContainerState {
    match status.state.as_ref() {
        Some(state) if state.running.is_some() => ContainerState::Running,
        Some(state) if state.waiting.is_some() => ContainerState::Waiting,
        Some(state) => state
            .terminated
            .as_ref()
            .map_or(ContainerState::Unknown, |t| ContainerState::Terminated {
                exit_code: t.exit_code,
            }),
        None => ContainerState::Unknown,
    }
}
