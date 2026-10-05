//! The cluster reads a forward needs, behind a trait so the session logic runs against a
//! scripted cluster in unit tests and against kube in production.

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::{Pod, Service};
use kube::api::ListParams;
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client};
use oxikube_domain::OxiResult;
use oxikube_domain::redact::redact;

use super::plan::{PodInfo, PodSelector, ServiceInfo};
use super::pods::{PodSet, pod_info, service_info};
use crate::auth::classify;

/// Reads of services and pods.
#[async_trait]
pub(super) trait Cluster: Send + Sync {
    /// The service `namespace/name`; `NotFound` when absent.
    async fn service(&self, namespace: &str, name: &str) -> OxiResult<ServiceInfo>;

    /// The pods matching `selector` right now.
    async fn pods(&self, namespace: &str, selector: &PodSelector) -> OxiResult<Vec<PodInfo>>;

    /// The matching pods again after every change. The stream is lazy, never ends on its own
    /// and survives API errors (it retries with backoff).
    fn watch_pods(
        &self,
        namespace: &str,
        selector: &PodSelector,
    ) -> BoxStream<'static, Vec<PodInfo>>;
}

/// [`Cluster`] over one kube client.
pub(super) struct KubeCluster {
    client: Client,
}

impl KubeCluster {
    pub(super) fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Cluster for KubeCluster {
    async fn service(&self, namespace: &str, name: &str) -> OxiResult<ServiceInfo> {
        let api: Api<Service> = Api::namespaced(self.client.clone(), namespace);
        let service = api.get(name).await.map_err(|e| classify(&e))?;
        Ok(service_info(&service))
    }

    async fn pods(&self, namespace: &str, selector: &PodSelector) -> OxiResult<Vec<PodInfo>> {
        let api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut params = ListParams::default();
        params.field_selector = selector.field_selector();
        params.label_selector = selector.label_selector();
        let list = api.list(&params).await.map_err(|e| classify(&e))?;
        Ok(list.items.iter().filter_map(pod_info).collect())
    }

    fn watch_pods(
        &self,
        namespace: &str,
        selector: &PodSelector,
    ) -> BoxStream<'static, Vec<PodInfo>> {
        let api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut config = watcher::Config::default();
        config.field_selector = selector.field_selector();
        config.label_selector = selector.label_selector();
        let mut set = PodSet::default();
        watcher(api, config)
            .default_backoff()
            .filter_map(move |event| {
                futures::future::ready(match event {
                    Ok(event) => set.apply(event),
                    Err(err) => {
                        tracing::debug!(
                            error = %redact(&err.to_string()),
                            "port-forward pod watch error; retrying with backoff"
                        );
                        None
                    }
                })
            })
            .boxed()
    }
}
