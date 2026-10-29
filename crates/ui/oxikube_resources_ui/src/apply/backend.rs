//! [`ApplyBackend`]: what the panel needs from outside. The app implements it over the services
//! ([`ServiceBackend`](super::ServiceBackend)); a test over a script.

use std::path::PathBuf;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oxikube_app::apply::{ApplyError, ApplyOptions, ClusterPlan, ObjectResult, PlanTarget};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kustomize::KustomizeTool;
use oxikube_domain::manifests::{ManifestDoc, ManifestError};

use super::source::ApplySource;

/// What loading a source found. The `Debug` form names objects, never their content.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    /// The objects, in file order then document order.
    pub docs: Vec<ManifestDoc>,
    /// Documents and files that did not parse.
    pub errors: Vec<ManifestError>,
    /// Entries skipped on purpose.
    pub notes: Vec<String>,
    /// Manifest files read.
    pub files: usize,
    /// The kustomize tool that built the source, for a kustomize source.
    pub tool: Option<KustomizeTool>,
    /// What kustomize wrote to stderr while succeeding.
    pub stderr: String,
}

/// The results of a dry run or an apply, one per object as it is decided; a refusal of the whole
/// plan is one `Err`, the last.
pub type ResultStream = BoxStream<'static, Result<ObjectResult, ApplyError>>;

/// The panel's view of the app. Futures and streams are `Send`: the panel runs them on the
/// Kubernetes runtime, never on the UI thread. The plain methods are cheap reads of state the app
/// holds and may be called on the UI thread.
pub trait ApplyBackend: 'static {
    /// Reads `source`: the files over `FsPort`, or the build through kustomize.
    ///
    /// # Errors
    ///
    /// A missing path, a limit hit, or (kustomize) the build's failure with its stderr.
    fn load(&self, source: ApplySource) -> BoxFuture<'static, OxiResult<Loaded>>;

    /// Orders `docs` for `target` (kind order, namespace defaults, edges).
    fn plan(&self, docs: Vec<ManifestDoc>, target: PlanTarget) -> BoxFuture<'static, ClusterPlan>;

    /// The server's dry run of every object of `plan`.
    fn dry_run(&self, plan: ClusterPlan, options: ApplyOptions) -> ResultStream;

    /// Applies `plan`: one guarded dispatch, confirmed by the panel before it is called.
    fn apply(&self, plan: ClusterPlan, options: ApplyOptions) -> ResultStream;

    /// The kustomize tool, `None` when neither `kustomize` nor `kubectl kustomize` exists.
    fn kustomize_tool(&self) -> BoxFuture<'static, Option<KustomizeTool>>;

    /// The text of the file at `path`, for "Open in editor".
    fn read_text(&self, path: PathBuf) -> BoxFuture<'static, OxiResult<String>>;

    /// The directory last loaded for `cluster`.
    fn last_directory(&self, cluster: ClusterId) -> BoxFuture<'static, Option<PathBuf>>;

    /// Remembers `path` as the directory last loaded for `cluster` (a path, never a secret).
    fn remember_directory(&self, cluster: ClusterId, path: PathBuf) -> BoxFuture<'static, ()>;

    /// Whether `cluster` is in read-only mode.
    fn read_only(&self, cluster: &ClusterId) -> bool;

    /// The context name to show for `cluster`.
    fn cluster_label(&self, cluster: &ClusterId) -> String;

    /// The one namespace the cluster's tab has selected, `None` for several or all.
    fn selected_namespace(&self, cluster: &ClusterId) -> Option<String>;
}
