//! The apply panel of the main window (E20-S05): `apply::Directory` and `apply::Kustomize` open
//! it in the cluster's tab, over the app's services.
//!
//! * [`start_views`] opens the panel for each request on the UI thread
//!   (`oxikube_resources_ui::apply::ApplyViews`). The panel's backend is the app's
//!   [`ApplyService`] (so every dry run and apply is `resource::ApplyPlan` through the guard), the
//!   `FsPort` that reads the manifests, the one [`KustomizeService`] (E20-S04), the state port
//!   (the last directory per cluster) and the sessions (read-only mode, the selected namespace).
//! * "Open in editor" sends the document to the window's manifest editors
//!   ([`EditorRequest::OpenText`]).
//!
//! A user reaches it from the command palette ("Apply: Directory or File", "Apply: Kustomize
//! Directory", which follows kustomize's detection), or an agent's `k8s.apply_directory`.

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::kustomize::KustomizeService;
use oxikube_app::{ApplyService, ClusterSessionManager};
use oxikube_editor::view::{EditorRequest, EditorViewSink, OpenText};
use oxikube_ports::{FsPort, StatePort};
use oxikube_resources_ui::apply::{
    ApplyRequest, ApplyViews, ApplyViewsDeps, EditorDocument, ServiceBackend,
};
use oxikube_workspace::ClusterTabs;

/// The services behind the panel.
pub struct PanelServices {
    /// Dry runs and applies, through the guard.
    pub apply: ApplyService,
    /// Reads manifests.
    pub fs: Arc<dyn FsPort>,
    /// Builds kustomize overlays.
    pub kustomize: KustomizeService,
    /// The last directory per cluster.
    pub state: Arc<dyn StatePort>,
    /// Read-only mode and the selected namespace.
    pub sessions: ClusterSessionManager,
}

/// Starts the window's apply panels over `requests` (the bus's `apply::*` handlers). Keep the task
/// for the life of the window.
pub fn start_views(
    services: PanelServices,
    editors: EditorViewSink,
    tabs: WeakEntity<ClusterTabs>,
    requests: UnboundedReceiver<ApplyRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let backend = ServiceBackend::new(
        services.apply,
        services.fs,
        Some(services.kustomize),
        services.state,
        services.sessions,
    );
    let deps = ApplyViewsDeps {
        backend: Rc::new(backend),
        editor: Rc::new(move |document: EditorDocument, _| {
            let request = EditorRequest::OpenText(OpenText {
                cluster: document.cluster,
                title: document.title,
                text: document.text,
                line: document.line,
            });
            if editors.send(request).is_err() {
                tracing::debug!("open in editor: the window with the editors is gone");
            }
        }),
        tabs,
    };
    ApplyViews::start(deps, requests, window, cx)
}
