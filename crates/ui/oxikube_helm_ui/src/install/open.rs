//! Opening the install tab: each `helm::InstallChart` (the catalog's Install button, the palette's
//! "Helm: Install Chart", an agent's `helm.install_chart`) opens it in the active cluster's tab.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use gpui::{App, AppContext as _, SharedString, Task, WeakEntity, Window};
use oxikube_domain::helm::ChartRef;
use oxikube_workspace::{ClusterTabs, OpenOptions};

use super::backend::InstallBackend;
use super::view::{HelmInstallView, InstallDeps, item_key};

/// What opening the tab needs.
#[derive(Clone)]
pub struct HelmInstallViewsDeps {
    /// Where the tab reads and installs.
    pub backend: Rc<dyn InstallBackend>,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's install tabs.
pub struct HelmInstallViews;

impl HelmInstallViews {
    /// Opens a tab for each chart `requests` yields (the charts service's install requests), on
    /// the UI thread. Keep the returned task for the life of the window.
    pub fn start(
        deps: HelmInstallViewsDeps,
        mut requests: BoxStream<'static, ChartRef>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(chart) = requests.next().await {
                match cx.update(|window, cx| Self::open(&deps, chart, window, cx)) {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!("helm::InstallChart: no cluster tab to install into");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the install tab of `chart` in the active cluster's tab (its open one when there is
    /// one, at the version asked for) and brings the cluster tab forward. `false` when no cluster
    /// tab is open.
    pub fn open(
        deps: &HelmInstallViewsDeps,
        chart: ChartRef,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(tabs) = deps.tabs.upgrade() else {
            return false;
        };
        let Some(cluster) = tabs
            .read(cx)
            .active()
            .cloned()
            .or_else(|| tabs.read(cx).clusters(cx).into_iter().next())
        else {
            return false;
        };
        let Some(tab) = tabs.read(cx).tab(&cluster).cloned() else {
            return false;
        };
        let workspace = tab.read(cx).workspace().clone();
        let label = tab.read(cx).info().title.clone();
        let key = SharedString::from(item_key(&chart));
        match workspace.read(cx).find_item_by_key(&key, cx) {
            Some(item) => {
                let open = workspace.read(cx).items_of_type::<HelmInstallView>();
                if let Some(view) = open.into_iter().find(|view| view.entity_id() == item) {
                    let version = chart.version.clone();
                    view.update(cx, |view, cx| view.set_version(&version, cx));
                }
                workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx));
            }
            None => {
                let page = InstallDeps {
                    backend: deps.backend.clone(),
                    cluster: cluster.clone(),
                    cluster_label: label,
                };
                let view = cx.new(|cx| HelmInstallView::new(page, chart, window, cx));
                let options = OpenOptions {
                    focus: true,
                    reuse_existing: true,
                    ..OpenOptions::default()
                };
                workspace.update(cx, |ws, cx| {
                    ws.open_item_with(Box::new(view), options, window, cx);
                });
            }
        }
        tabs.update(cx, |tabs, cx| {
            tabs.activate(&cluster, window, cx);
        });
        true
    }
}
