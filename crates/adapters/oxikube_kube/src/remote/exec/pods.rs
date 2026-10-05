//! The pod reads and writes exec needs beyond the stream itself, behind a trait so the node
//! shell and debug flows run against a scripted cluster in unit tests and against kube in
//! production.

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{DeleteParams, ListParams, ObjectMeta, Patch, PatchParams, PostParams};
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::watch_object;
use kube::{Api, Client};
use oxikube_domain::{OxiError, OxiResult};
use serde_json::Value;

use super::wait::{Container, Readiness, readiness};
use crate::auth::{classify, redacted_line};

/// What a failed exec needs to know about a pod to say what went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PodShape {
    /// `status.phase`, empty when unset.
    pub(super) phase: String,
    /// The pod has a deletion timestamp.
    pub(super) deleting: bool,
    /// Names of its containers, init containers and ephemeral containers.
    pub(super) containers: Vec<String>,
}

/// A pod found by label, for the leftover sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PodStamp {
    pub(super) name: String,
    /// Seconds since the epoch.
    pub(super) created: i64,
}

/// Pod operations of the exec adapter.
#[async_trait]
pub(super) trait Pods: Send + Sync {
    /// Creates the pod `manifest` (with `generateName`) in `namespace`; returns its name.
    async fn create(&self, namespace: &str, manifest: &Value) -> OxiResult<String>;

    /// Deletes a pod without a grace period. A pod that is already gone is not an error.
    async fn delete(&self, namespace: &str, name: &str) -> OxiResult<()>;

    /// The pods matching a label selector.
    async fn list(&self, namespace: &str, label_selector: &str) -> OxiResult<Vec<PodStamp>>;

    /// The shape of a pod; `None` when it does not exist.
    async fn shape(&self, namespace: &str, name: &str) -> OxiResult<Option<PodShape>>;

    /// Adds an ephemeral container with a strategic merge patch of `ephemeralcontainers`.
    async fn add_ephemeral_container(
        &self,
        namespace: &str,
        pod: &str,
        patch: &Value,
    ) -> OxiResult<()>;

    /// Waits until `container` of `pod` is running; an error as soon as it cannot become
    /// running (the image cannot be pulled, the pod ended), or `Timeout` after `timeout`.
    async fn wait_running(
        &self,
        namespace: &str,
        pod: &str,
        container: &Container,
        timeout: Duration,
    ) -> OxiResult<()>;
}

/// [`Pods`] over one kube client.
pub(super) struct KubePods {
    client: Client,
}

impl KubePods {
    pub(super) fn new(client: Client) -> Self {
        Self { client }
    }

    fn api(&self, namespace: &str) -> Api<Pod> {
        Api::namespaced(self.client.clone(), namespace)
    }
}

#[async_trait]
impl Pods for KubePods {
    async fn create(&self, namespace: &str, manifest: &Value) -> OxiResult<String> {
        let pod: Pod = serde_json::from_value(manifest.clone())
            .map_err(|err| OxiError::validation(format!("not a valid pod: {err}")))?;
        let created = self
            .api(namespace)
            .create(&PostParams::default(), &pod)
            .await
            .map_err(|err| classify(&err))?;
        let ObjectMeta { name, .. } = created.metadata;
        name.ok_or_else(|| OxiError::internal("the created pod has no name"))
    }

    async fn delete(&self, namespace: &str, name: &str) -> OxiResult<()> {
        let params = DeleteParams::default().grace_period(0);
        match self.api(namespace).delete(name, &params).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(status)) if status.code == 404 => Ok(()),
            Err(err) => Err(classify(&err)),
        }
    }

    async fn list(&self, namespace: &str, label_selector: &str) -> OxiResult<Vec<PodStamp>> {
        let params = ListParams::default().labels(label_selector);
        let list = self
            .api(namespace)
            .list(&params)
            .await
            .map_err(|err| classify(&err))?;
        Ok(list
            .items
            .into_iter()
            .filter_map(|pod| {
                Some(PodStamp {
                    name: pod.metadata.name?,
                    created: pod
                        .metadata
                        .creation_timestamp
                        .map_or(0, |t| t.0.as_second()),
                })
            })
            .collect())
    }

    async fn shape(&self, namespace: &str, name: &str) -> OxiResult<Option<PodShape>> {
        let pod = self
            .api(namespace)
            .get_opt(name)
            .await
            .map_err(|err| classify(&err))?;
        Ok(pod.map(|pod| {
            let spec = pod.spec.unwrap_or_default();
            let containers = spec
                .containers
                .into_iter()
                .chain(spec.init_containers.into_iter().flatten())
                .map(|c| c.name)
                .chain(
                    spec.ephemeral_containers
                        .into_iter()
                        .flatten()
                        .map(|c| c.name),
                )
                .collect();
            PodShape {
                phase: pod.status.and_then(|s| s.phase).unwrap_or_default(),
                deleting: pod.metadata.deletion_timestamp.is_some(),
                containers,
            }
        }))
    }

    async fn add_ephemeral_container(
        &self,
        namespace: &str,
        pod: &str,
        patch: &Value,
    ) -> OxiResult<()> {
        self.api(namespace)
            .patch_ephemeral_containers(pod, &PatchParams::default(), &Patch::Strategic(patch))
            .await
            .map(drop)
            .map_err(|err| classify(&err))
    }

    async fn wait_running(
        &self,
        namespace: &str,
        pod: &str,
        container: &Container,
        timeout: Duration,
    ) -> OxiResult<()> {
        // A watch that fails is retried with backoff; only the timeout ends the wait.
        let events = watch_object(self.api(namespace), pod)
            .default_backoff()
            .filter_map(|event| {
                futures::future::ready(match event {
                    Ok(pod) => pod,
                    Err(err) => {
                        tracing::debug!(
                            error = %redacted_line(&err.to_string()),
                            "pod watch error while waiting for a container; retrying"
                        );
                        None
                    }
                })
            });
        let settled = async {
            let mut events = std::pin::pin!(events);
            while let Some(found) = events.next().await {
                if readiness(&found, container) != Readiness::Waiting {
                    return Some(found);
                }
            }
            None
        };
        let pod_object = match tokio::time::timeout(timeout, settled).await {
            Err(_) => {
                return Err(OxiError::timeout(format!(
                    "{} of pod {namespace}/{pod} did not start within {}s",
                    container.name(),
                    timeout.as_secs()
                )));
            }
            Ok(found) => found,
        };
        match pod_object.map(|pod| readiness(&pod, container)) {
            Some(Readiness::Running) => Ok(()),
            Some(Readiness::Failed(reason)) => Err(OxiError::conflict(format!(
                "{} of pod {namespace}/{pod} cannot start: {reason}",
                container.name()
            ))),
            _ => Err(OxiError::not_found(format!(
                "pod {namespace}/{pod} disappeared while starting"
            ))),
        }
    }
}
