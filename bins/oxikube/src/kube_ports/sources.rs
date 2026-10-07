//! [`LazyKubeSources`]: the kubeconfig catalog (`oxikube_kube::sources`), built on first use.
//!
//! Start-up must not read kubeconfig files (E05-S13, `startup::deferred`): this port is built
//! before the first frame and holds only the inputs. The adapter itself
//! ([`KubeconfigSources`], with its file watcher) is built by the first port call, which always
//! runs on the Tokio bridge (the catalog's read, a connect, the sources screen), and every later
//! call goes to it. The process environment (`KUBECONFIG`, the home directory, the in-cluster
//! files) is read at that moment too, off the UI thread.
//!
//! Subscribers may come earlier (the catalog view subscribes when it is built, before anything
//! was read): they are kept here and fed from the adapter's own stream once it exists.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_kube::kubeconfig::{Env, LoadedKubeconfig};
use oxikube_kube::sources::{KubeconfigSources, SourcesConfig};
use oxikube_ports::{
    ClusterContext, ClusterSource, ClusterSourcePort, SecretStorePort, SourceStatus,
    SourcesChanged, UserSource, UserSourceKind,
};
use parking_lot::Mutex;
use tokio::sync::OnceCell;

type Subscribers = Arc<Mutex<Vec<mpsc::UnboundedSender<SourcesChanged>>>>;

/// The kubeconfig-backed `ClusterSourcePort`, built on first use. See the module docs.
pub struct LazyKubeSources {
    adapter: OnceCell<Arc<KubeconfigSources>>,
    /// The user's source list as settings had it at start-up (`kubeconfig.sources`); the sources
    /// screen replaces it at run time through `set_user_sources`.
    initial: Vec<UserSource>,
    secrets: Arc<dyn SecretStorePort>,
    /// Start the adapter's file watcher. Off in tests, which drive `reload()` themselves.
    watch: bool,
    subscribers: Subscribers,
}

impl std::fmt::Debug for LazyKubeSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyKubeSources")
            .field("built", &self.adapter.initialized())
            .field("sources", &self.initial.len())
            .finish_non_exhaustive()
    }
}

impl LazyKubeSources {
    /// The port, reading nothing yet. `initial` is the user's source list from settings,
    /// `secrets` keeps pasted kubeconfig text for the adapter, `watch` starts its file watcher.
    pub fn new(initial: Vec<UserSource>, secrets: Arc<dyn SecretStorePort>, watch: bool) -> Self {
        Self {
            adapter: OnceCell::new(),
            initial,
            secrets,
            watch,
            subscribers: Arc::default(),
        }
    }

    /// Whether the adapter was built (a port call ran).
    #[cfg(test)]
    pub(crate) fn is_built(&self) -> bool {
        self.adapter.initialized()
    }

    /// The loader result of the current catalog, read first if nothing was read yet, for the
    /// client pools. Holds credentials: never log it.
    ///
    /// # Errors
    ///
    /// When the adapter cannot be built or the sources cannot be read.
    pub async fn loaded(&self) -> OxiResult<Arc<LoadedKubeconfig>> {
        let adapter = self.adapter().await?;
        // Loads the catalog when nothing was loaded yet; a no-op read otherwise.
        adapter.contexts().await?;
        adapter
            .loaded()
            .ok_or_else(|| OxiError::internal("the kubeconfig catalog was not loaded"))
    }

    async fn adapter(&self) -> OxiResult<&Arc<KubeconfigSources>> {
        self.adapter.get_or_try_init(|| self.build()).await
    }

    async fn build(&self) -> OxiResult<Arc<KubeconfigSources>> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(OxiError::internal(
                "the kubeconfig sources were first used off the Tokio bridge",
            ));
        }
        // `Env::from_process` stats the in-cluster files: not on the caller's thread.
        let env = tokio::task::spawn_blocking(Env::from_process)
            .await
            .map_err(|err| OxiError::internal("reading the environment failed").with_source(err))?;
        let config = SourcesConfig {
            include_default: self
                .initial
                .iter()
                .any(|s| s.kind == UserSourceKind::Default),
            extra_paths: self
                .initial
                .iter()
                .filter(|s| s.kind != UserSourceKind::Default)
                .filter_map(|s| s.path.clone())
                .collect(),
            watch: self.watch,
            ..SourcesConfig::new(env)
        };
        let adapter = Arc::new(KubeconfigSources::new(config, self.secrets.clone())?);
        let mut changes = adapter.subscribe();
        let subscribers = self.subscribers.clone();
        // Ends when the adapter (and so its sender) is dropped with this port.
        tokio::spawn(async move {
            while let Some(diff) = changes.next().await {
                subscribers
                    .lock()
                    .retain(|tx| tx.unbounded_send(diff.clone()).is_ok());
            }
        });
        tracing::debug!(watch = self.watch, "kubeconfig sources built on first use");
        Ok(adapter)
    }
}

#[async_trait]
impl ClusterSourcePort for LazyKubeSources {
    async fn sources(&self) -> OxiResult<Vec<ClusterSource>> {
        self.adapter().await?.sources().await
    }

    async fn contexts(&self) -> OxiResult<Vec<ClusterContext>> {
        self.adapter().await?.contexts().await
    }

    fn subscribe(&self) -> BoxStream<'static, SourcesChanged> {
        let (tx, rx) = mpsc::unbounded();
        self.subscribers.lock().push(tx);
        rx.boxed()
    }

    async fn reload(&self) -> OxiResult<SourcesChanged> {
        self.adapter().await?.reload().await
    }

    async fn set_user_sources(&self, sources: &[UserSource]) -> OxiResult<SourcesChanged> {
        self.adapter().await?.set_user_sources(sources).await
    }

    async fn source_statuses(&self) -> OxiResult<Vec<SourceStatus>> {
        self.adapter().await?.source_statuses().await
    }

    async fn validate_kubeconfig(&self, text: &str) -> OxiResult<usize> {
        self.adapter().await?.validate_kubeconfig(text).await
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::*;
    use crate::kube_ports::MemorySecrets;

    /// How long a test waits for a change to arrive.
    const CHANGE_TIMEOUT: Duration = Duration::from_secs(5);

    const KUBECONFIG: &str = r#"
apiVersion: v1
kind: Config
clusters:
- name: c
  cluster: { server: "https://127.0.0.1:1" }
users:
- name: u
  user: { token: "not-a-real-token" }
contexts:
- name: one
  context: { cluster: c, user: u }
"#;

    fn port(file: &Path) -> LazyKubeSources {
        LazyKubeSources::new(
            vec![UserSource::file(file)],
            Arc::new(MemorySecrets::default()),
            false,
        )
    }

    #[tokio::test]
    async fn nothing_is_read_before_the_first_call_and_the_user_files_are_read_then() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config");
        std::fs::write(&file, KUBECONFIG).unwrap();
        let port = port(&file);
        assert!(!port.is_built());

        let contexts = port.contexts().await.unwrap();
        assert!(port.is_built());
        assert!(
            contexts.iter().any(|c| c.context.as_str() == "one"),
            "the user's file is read on first use"
        );
        let loaded = port.loaded().await.unwrap();
        assert!(loaded.merged.contexts.iter().any(|c| c.name == "one"));
    }

    #[tokio::test]
    async fn an_early_subscriber_gets_the_changes_of_the_adapter_built_later() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config");
        std::fs::write(&file, KUBECONFIG).unwrap();
        let port = port(&file);
        let mut changes = port.subscribe();
        port.contexts().await.unwrap();

        std::fs::write(&file, KUBECONFIG.replace("name: one", "name: two")).unwrap();
        port.reload().await.unwrap();
        let diff = tokio::time::timeout(CHANGE_TIMEOUT, changes.next())
            .await
            .expect("a change arrives")
            .expect("the stream is open");
        assert!(!diff.is_empty());
    }

    #[test]
    fn a_first_call_off_tokio_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let port = port(&dir.path().join("config"));
        let err = futures::executor::block_on(port.contexts()).unwrap_err();
        assert!(err.to_string().contains("Tokio"), "{err}");
        assert!(!port.is_built());
    }
}
