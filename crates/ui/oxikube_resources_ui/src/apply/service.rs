//! [`ServiceBackend`]: the panel over the app's services. Nothing here writes to a cluster: the
//! dry run and the apply are [`ApplyService`] dispatches of `resource::ApplyPlan`, which the
//! `MutationGuard` checks, confirms and audits.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use oxikube_app::ClusterSessionManager;
use oxikube_app::apply::{
    ApplyOptions, ApplyService, ClusterPlan, LoadLimits, PlanTarget, load_path,
};
use oxikube_app::guard::{ConfirmationProvider, ConfirmationRequest, Decision};
use oxikube_app::kustomize::KustomizeService;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kustomize::KustomizeTool;
use oxikube_domain::manifests::ManifestDoc;
use oxikube_domain::safety::ConfirmTier;
use oxikube_ports::{FsPort, StateKey, StatePort, StatePortExt as _};

use super::backend::{ApplyBackend, Loaded, ResultStream};
use super::source::{ApplySource, SourceKind};

/// The state key of the directory last loaded for a cluster (a path, never a secret).
fn directory_key(cluster: &ClusterId) -> Option<StateKey> {
    StateKey::new(format!("apply.last_dir.{}", cluster.as_str())).ok()
}

/// The panel's answer to the guard's confirmation: the user confirmed in the panel (its bar shows
/// what the dry run said), so a one-click request is confirmed here. A request for typing a name
/// is declined: the panel does not ask for one, and an apply never rates that high.
struct PanelConsent;

#[async_trait]
impl ConfirmationProvider for PanelConsent {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match request.tier {
            ConfirmTier::Simple => Decision::confirm(),
            other => {
                tracing::warn!(?other, "the apply panel cannot answer this confirmation");
                Decision::Decline
            }
        }
    }
}

/// The panel over [`ApplyService`], `FsPort`, the kustomize service, the state port and the
/// sessions.
#[derive(Clone)]
pub struct ServiceBackend {
    apply: ApplyService,
    fs: Arc<dyn FsPort>,
    kustomize: Option<KustomizeService>,
    state: Arc<dyn StatePort>,
    sessions: ClusterSessionManager,
}

impl ServiceBackend {
    /// A backend over the app's services. `kustomize` is `None` when the app has none.
    pub fn new(
        apply: ApplyService,
        fs: Arc<dyn FsPort>,
        kustomize: Option<KustomizeService>,
        state: Arc<dyn StatePort>,
        sessions: ClusterSessionManager,
    ) -> Self {
        Self {
            apply,
            fs,
            kustomize,
            state,
            sessions,
        }
    }
}

impl ApplyBackend for ServiceBackend {
    fn load(&self, source: ApplySource) -> BoxFuture<'static, OxiResult<Loaded>> {
        let fs = self.fs.clone();
        let kustomize = self.kustomize.clone();
        async move {
            match source.kind {
                SourceKind::Manifests => {
                    let loaded = load_path(&fs, &source.path, LoadLimits::default()).await?;
                    Ok(Loaded {
                        docs: loaded.docs,
                        errors: loaded.errors,
                        notes: loaded
                            .notes
                            .into_iter()
                            .map(|note| format!("{}: {}", note.path.display(), note.message))
                            .collect(),
                        files: loaded.files,
                        ..Loaded::default()
                    })
                }
                SourceKind::Kustomize => {
                    let service = kustomize.ok_or_else(|| {
                        oxikube_domain::OxiError::unsupported("kustomize is not available")
                    })?;
                    let build = service.build(&source.path).await?;
                    Ok(Loaded {
                        docs: build.docs,
                        errors: build.errors,
                        files: 1,
                        tool: Some(build.tool),
                        stderr: build.stderr,
                        ..Loaded::default()
                    })
                }
            }
        }
        .boxed()
    }

    fn plan(&self, docs: Vec<ManifestDoc>, target: PlanTarget) -> BoxFuture<'static, ClusterPlan> {
        let apply = self.apply.clone();
        async move { apply.plan(docs, target).await }.boxed()
    }

    fn dry_run(&self, plan: ClusterPlan, options: ApplyOptions) -> ResultStream {
        self.apply.dry_run_plan(&plan, &options).boxed()
    }

    fn apply(&self, plan: ClusterPlan, options: ApplyOptions) -> ResultStream {
        self.apply
            .apply_plan(&plan, &options, Arc::new(PanelConsent))
            .boxed()
    }

    fn kustomize_tool(&self) -> BoxFuture<'static, Option<KustomizeTool>> {
        let kustomize = self.kustomize.clone();
        async move {
            match kustomize {
                Some(service) => service.tool().await,
                None => None,
            }
        }
        .boxed()
    }

    fn read_text(&self, path: PathBuf) -> BoxFuture<'static, OxiResult<String>> {
        let fs = self.fs.clone();
        async move {
            let bytes = fs.read(&path).await?;
            String::from_utf8(bytes)
                .map_err(|_| oxikube_domain::OxiError::validation("the file is not valid UTF-8"))
        }
        .boxed()
    }

    fn last_directory(&self, cluster: ClusterId) -> BoxFuture<'static, Option<PathBuf>> {
        let state = self.state.clone();
        async move {
            let key = directory_key(&cluster)?;
            state
                .kv_get_as::<String>(&key)
                .await
                .ok()
                .flatten()
                .map(PathBuf::from)
        }
        .boxed()
    }

    fn remember_directory(&self, cluster: ClusterId, path: PathBuf) -> BoxFuture<'static, ()> {
        let state = self.state.clone();
        async move {
            let Some(key) = directory_key(&cluster) else {
                return;
            };
            if let Err(error) = state.kv_set_as(&key, &path.display().to_string()).await {
                tracing::debug!(%error, "the apply panel could not remember its directory");
            }
        }
        .boxed()
    }

    fn read_only(&self, cluster: &ClusterId) -> bool {
        self.sessions
            .get(cluster)
            .is_some_and(|session| session.read_only())
    }

    fn cluster_label(&self, cluster: &ClusterId) -> String {
        self.sessions.get(cluster).map_or_else(
            || cluster.to_string(),
            |session| session.context().to_string(),
        )
    }

    fn selected_namespace(&self, cluster: &ClusterId) -> Option<String> {
        let session = self.sessions.get(cluster)?;
        let mut names = session.namespace_selection().names();
        let first = names.next()?;
        names.next().is_none().then(|| first.to_owned())
    }
}
