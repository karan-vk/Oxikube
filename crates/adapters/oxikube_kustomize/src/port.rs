//! [`KustomizeCli`]: the [`KustomizePort`] over the user's own binary.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::kustomize::{BuildOutput, KustomizeError, KustomizeTool};
use oxikube_ports::KustomizePort;
use tokio::sync::Mutex as AsyncMutex;

use crate::build::{args, clean_stderr, failure};
use crate::config::{KustomizeConfig, SearchPath};
use crate::detect::detect;
use crate::path::{Folders, login_shell_path};
use crate::run::run;

/// What detection found, kept until [`KustomizeCli::reconfigure`] or `refresh`.
#[derive(Default)]
struct Cache {
    folders: Option<Folders>,
    tool: Option<Option<KustomizeTool>>,
}

/// Builds overlays with `kustomize build` or `kubectl kustomize`.
///
/// Needs a Tokio runtime with the process and time drivers (`oxikube_runtime::spawn_kube`
/// provides it); never call it on the UI thread. Detection runs on first use and is cached;
/// the login shell is asked once per cache.
pub struct KustomizeCli {
    config: Mutex<Arc<KustomizeConfig>>,
    cache: AsyncMutex<Cache>,
}

impl KustomizeCli {
    /// An adapter with `config`.
    pub fn new(config: KustomizeConfig) -> Self {
        Self {
            config: Mutex::new(Arc::new(config)),
            cache: AsyncMutex::new(Cache::default()),
        }
    }

    /// Applies a changed configuration (the `kustomize` settings, hot reload). The next call
    /// detects again.
    pub async fn reconfigure(&self, config: KustomizeConfig) {
        *self.config.lock().unwrap_or_else(PoisonError::into_inner) = Arc::new(config);
        *self.cache.lock().await = Cache::default();
    }

    fn config(&self) -> Arc<KustomizeConfig> {
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The folders to search and the detected tool, detecting when `force` or not yet done.
    async fn resolve(&self, force: bool) -> (Folders, Option<KustomizeTool>) {
        let config = self.config();
        let mut cache = self.cache.lock().await;
        if force {
            cache.tool = None;
        }
        let folders = match &cache.folders {
            Some(folders) => folders.clone(),
            None => {
                let folders = folders_for(&config).await;
                cache.folders = Some(folders.clone());
                folders
            }
        };
        let tool = match &cache.tool {
            Some(tool) => tool.clone(),
            None => {
                let tool = detect(&config, &folders).await;
                match &tool {
                    Some(found) => tracing::debug!(tool = %found, "kustomize detected"),
                    None => tracing::debug!("no kustomize or kubectl found"),
                }
                cache.tool = Some(tool.clone());
                tool
            }
        };
        (folders, tool)
    }
}

async fn folders_for(config: &KustomizeConfig) -> Folders {
    match &config.search_path {
        SearchPath::Fixed(list) => Folders::fixed(list),
        SearchPath::Auto => {
            let login = login_shell_path(config.probe_timeout).await;
            let process = std::env::var_os("PATH");
            Folders::build(login.as_deref(), process.as_deref())
        }
    }
}

#[async_trait]
impl KustomizePort for KustomizeCli {
    async fn detect(&self) -> Option<KustomizeTool> {
        self.resolve(false).await.1
    }

    async fn refresh(&self) -> Option<KustomizeTool> {
        self.resolve(true).await.1
    }

    async fn build(&self, dir: &Path) -> OxiResult<BuildOutput> {
        let (folders, tool) = self.resolve(false).await;
        let tool = tool.ok_or(KustomizeError::NotInstalled)?;
        let timeout = self.config().build_timeout;
        // Only the directory is logged: stdout may hold `secretGenerator` values.
        tracing::debug!(dir = %dir.display(), tool = %tool, "kustomize build");
        let done = run(
            &tool.path,
            &args(tool.kind, dir),
            folders.as_path_var(),
            timeout,
        )
        .await?;
        if !done.status.success() {
            return Err(failure(tool.kind, done.status, &done.stderr).into());
        }
        Ok(BuildOutput {
            yaml: done.stdout,
            stderr: clean_stderr(&done.stderr),
        })
    }
}
