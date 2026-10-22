//! The Helm install tab of the main window (E16-S09): every `helm::InstallChart` (the charts
//! catalog's Install button, the palette's "Helm: Install Chart") opens it in the active cluster's
//! tab; its preview and install are `helm::Install` dispatches through the guard.
//!
//! * [`targets`] tells the `helm` adapter which kubeconfig context (and file) a cluster's install
//!   runs against: the session's context, in the file the catalog read it from.
//! * [`start_views`] opens a tab per install request (`oxikube_helm_ui::install::HelmInstallViews`).

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::{ChartsService, ClusterSessionManager, InstallFlow};
use oxikube_domain::ids::ClusterId;
use oxikube_helm::{HelmTarget, TargetResolver};
use oxikube_helm_ui::install::{FlowBackend, HelmInstallViews, HelmInstallViewsDeps};
use oxikube_workspace::ClusterTabs;

use crate::kube_ports::KubeconfigOrigins;

/// The kubeconfig context of each open session, in the file that defines it (`--kube-context`,
/// `--kubeconfig`). A cluster with no open session has none, so `helm` is never pointed at a
/// cluster the user did not open.
pub fn targets(
    sessions: ClusterSessionManager,
    origins: KubeconfigOrigins,
) -> Arc<dyn TargetResolver> {
    Arc::new(move |cluster: &ClusterId| {
        let session = sessions.get(cluster)?;
        let context = session.context().clone();
        let file = origins.file_of(&context);
        Some(HelmTarget::new(context, file))
    })
}

/// Starts the window's install tabs: one per chart the charts service is asked to install. Keep
/// the task for the life of the window.
pub fn start_views(
    flow: InstallFlow,
    charts: ChartsService,
    tabs: WeakEntity<ClusterTabs>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    // Every request after this point (the watch's current value is an older request).
    let mut watch = charts.install_requests();
    watch.mark_unchanged();
    let requests = futures::stream::unfold(watch, |mut watch| async move {
        watch.changed().await.ok()?;
        let chart = watch.borrow_and_update().clone();
        Some((chart, watch))
    })
    .filter_map(|chart| async move { chart })
    .boxed();
    let deps = HelmInstallViewsDeps {
        backend: Rc::new(FlowBackend::new(flow, charts)),
        tabs,
    };
    HelmInstallViews::start(deps, requests, window, cx)
}
