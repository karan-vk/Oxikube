//! [`SourcesConnector`]: the kube adapter's `ClusterConnectorPort`, kept in step with the
//! kubeconfig catalog.
//!
//! `oxikube_kube::KubeConnector` builds a connection's ports (pooled client, discovery, resource
//! reads and writes, Table feeds, logs, exec, port-forward, metrics, access review, the liveness
//! loop and the per-connection watch budget) from the kubeconfig its pools hold. The catalog is
//! [`LazyKubeSources`] and reloads at run time (files edited, sources added on the sources
//! screen), so before each connect this wrapper hands the catalog's current loader result to the
//! connector ([`KubeConnector::replace_loaded`]) when it changed since the last connect. A
//! connect therefore always uses the kubeconfig the catalog showed, and nothing is read before the
//! first connect.
//!
//! Each connection also gets its `DescribePort` here (E07-S06): deskribe over the connection's
//! client, `kubectl describe` as the fallback (pointed at the kubeconfig file that defines the
//! context), one [`Describer`] choosing between them by the shared [`DescribePreference`], which
//! the mount sets from the `describe` setting.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use kube::config::Kubeconfig;
use oxikube_describe::{
    DescribePreference, Describer, KubectlDescribe, KubectlTarget, NativeDescribe,
};
use oxikube_domain::OxiResult;
use oxikube_kube::kubeconfig::LoadedKubeconfig;
use oxikube_kube::{ConnectorConfig, DescribeConnection, KubeConnector, PoolConfig};
use oxikube_ports::{ClusterConnection, ClusterConnectorPort, ConnectRequest, DescribePort};
use parking_lot::Mutex;

use super::LazyKubeSources;

/// See the module docs.
pub struct SourcesConnector {
    sources: Arc<LazyKubeSources>,
    kube: KubeConnector,
    /// The loader result the connector's pools hold now (shared with the describe factory, which
    /// reads the file that defines a context from it).
    synced: Arc<Mutex<Option<Arc<LoadedKubeconfig>>>>,
}

impl std::fmt::Debug for SourcesConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourcesConnector")
            .field("kube", &self.kube)
            .finish_non_exhaustive()
    }
}

impl SourcesConnector {
    /// A connector for the contexts of `sources`, with the default client and connection
    /// settings; every connection's describer follows `describe`.
    pub fn new(sources: Arc<LazyKubeSources>, describe: DescribePreference) -> Self {
        // Empty until the first connect hands it the catalog.
        let kube = KubeConnector::new(
            Kubeconfig::default(),
            PoolConfig::default(),
            ConnectorConfig::default(),
        );
        let synced: Arc<Mutex<Option<Arc<LoadedKubeconfig>>>> = Arc::default();
        let origins = synced.clone();
        kube.set_describe_factory(Arc::new(move |connection: DescribeConnection| {
            let context = &connection.context;
            // The in-cluster context has no file `kubectl` could be pointed at.
            let kubeconfig = origins
                .lock()
                .as_ref()
                .filter(|loaded| !loaded.is_in_cluster(context))
                .and_then(|loaded| loaded.origin(context).map(Path::to_path_buf));
            describer(connection, &describe, kubeconfig)
        }));
        Self {
            sources,
            kube,
            synced,
        }
    }

    /// The kube connector behind this one (its watch budgets, `super::WatchBudgets`).
    pub fn kube(&self) -> &KubeConnector {
        &self.kube
    }

    /// Hands the catalog's current kubeconfig to the pools when it changed since the last call.
    async fn sync(&self) -> OxiResult<()> {
        let loaded = self.sources.loaded().await?;
        let mut synced = self.synced.lock();
        if synced.as_ref().is_some_and(|s| Arc::ptr_eq(s, &loaded)) {
            return Ok(());
        }
        let dropped = self.kube.replace_loaded(loaded.clone());
        if !dropped.is_empty() {
            tracing::debug!(
                contexts = dropped.len(),
                "kubeconfig changed: pooled clients dropped"
            );
        }
        *synced = Some(loaded);
        Ok(())
    }
}

/// The `DescribePort` of one connection: deskribe over its client, `kubectl` for the rest.
fn describer(
    connection: DescribeConnection,
    preference: &DescribePreference,
    kubeconfig: Option<PathBuf>,
) -> Arc<dyn DescribePort> {
    let native = NativeDescribe::new(connection.client, connection.discovery.clone());
    let kubectl = KubectlDescribe::new(
        connection.discovery,
        preference.clone(),
        KubectlTarget {
            context: connection.context,
            kubeconfig,
        },
    );
    Arc::new(Describer::new(
        Arc::new(native),
        Arc::new(kubectl),
        preference.clone(),
    ))
}

#[async_trait]
impl ClusterConnectorPort for SourcesConnector {
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection> {
        self.sync().await?;
        self.kube.connect(request).await
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ErrorKind;
    use oxikube_domain::ids::{ClusterId, ContextName};
    use oxikube_ports::{
        ClusterSourcePort as _, ExecInteractivity, HealthReporter, HealthSignal, UserSource,
    };

    use super::*;
    use crate::kube_ports::MemorySecrets;

    struct NoReports;

    impl HealthReporter for NoReports {
        fn report(&self, _: HealthSignal) {}
    }

    fn request(context: &str) -> ConnectRequest {
        let context = ContextName::new(context);
        ConnectRequest {
            cluster: ClusterId::new("test", &context),
            context,
            exec_interactivity: ExecInteractivity::Never,
            health: Arc::new(NoReports),
        }
    }

    fn kubeconfig(context: &str) -> String {
        format!(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster: {{ server: \"https://127.0.0.1:1\" }}\n\
             users:\n- name: u\n  user: {{ token: \"not-a-real-token\" }}\n\
             contexts:\n- name: {context}\n  context: {{ cluster: c, user: u }}\n"
        )
    }

    #[tokio::test]
    async fn a_connect_uses_the_catalog_as_it_is_now() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config");
        std::fs::write(&file, kubeconfig("first")).unwrap();
        let sources = Arc::new(LazyKubeSources::new(
            vec![UserSource::file(&file)],
            Arc::new(MemorySecrets::default()),
            false,
        ));
        let connector = SourcesConnector::new(sources.clone(), DescribePreference::default());
        assert!(!sources.is_built(), "nothing read before the first connect");

        let missing = connector.connect(request("second")).await.err();
        assert_eq!(missing.map(|e| e.kind()), Some(ErrorKind::NotFound));
        // The client build needs no network; the liveness loop fails in the background.
        let first = connector.connect(request("first")).await;
        assert!(first.is_ok(), "the catalog's context connects");

        std::fs::write(&file, kubeconfig("second")).unwrap();
        sources.reload().await.unwrap();
        let second = connector.connect(request("second")).await;
        assert!(second.is_ok(), "a context added by a reload connects");
    }
}
