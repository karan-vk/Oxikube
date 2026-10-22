//! Opening the upgrade tab: each `helm::UpgradeRelease` (the sidebar's Helm > Upgrade entry, the
//! palette's "Helm: Upgrade Release...", a release's Upgrade button, an agent's
//! `helm.upgrade_release`) opens it in the active cluster's tab.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use gpui::{App, AppContext as _, SharedString, Task, WeakEntity, Window};
use oxikube_app::UpgradeTarget;
use oxikube_domain::command::CommandId;
use oxikube_domain::helm::ReleaseRef;
use oxikube_workspace::sidebar::{SidebarEntry, SidebarRegistry, SidebarTarget};
use oxikube_workspace::{ClusterTabs, OpenOptions};

use super::backend::UpgradeBackend;
use super::view::{HelmUpgradeView, UpgradeDeps, item_key};

/// The id of the sidebar section the entry joins (the Helm section of the core sidebar).
pub const SECTION_ID: &str = "helm";

/// The entry's id in that section.
pub const ENTRY_ID: &str = "upgrade";

/// The "Upgrade" entry: runs `helm::UpgradeRelease` without a release, so the tab lists the
/// cluster's releases to pick from. It has no access requirement of its own: the tab reads the
/// releases and says what it cannot.
pub fn sidebar_entry() -> SidebarEntry {
    SidebarEntry {
        id: ENTRY_ID.into(),
        title: "Upgrade".into(),
        requires: Vec::new(),
        target: Some(SidebarTarget::Command(CommandId::HELM_UPGRADE_RELEASE)),
        kind: None,
    }
}

/// Adds the entry to the Helm section. Idempotent; `false` when the section is not registered.
pub fn register_sidebar(cx: &mut App) -> bool {
    SidebarRegistry::add_entry(cx, SECTION_ID, sidebar_entry())
}

/// What opening the tab needs.
#[derive(Clone)]
pub struct HelmUpgradeViewsDeps {
    /// Where the tab reads and upgrades.
    pub backend: Rc<dyn UpgradeBackend>,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's upgrade tabs.
pub struct HelmUpgradeViews;

impl HelmUpgradeViews {
    /// Opens a tab for each target `requests` yields (the app's upgrade requests), on the UI
    /// thread. Keep the returned task for the life of the window.
    pub fn start(
        deps: HelmUpgradeViewsDeps,
        mut requests: BoxStream<'static, UpgradeTarget>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(target) = requests.next().await {
                match cx.update(|window, cx| Self::open(&deps, &target, window, cx)) {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!("helm::UpgradeRelease: no cluster tab to upgrade in");
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the upgrade tab of `target` in the active cluster's tab (its open one when there is
    /// one) and brings the cluster tab forward. `false` when no cluster tab is open.
    pub fn open(
        deps: &HelmUpgradeViewsDeps,
        target: &UpgradeTarget,
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
        let release = target
            .release
            .as_ref()
            .map(|(namespace, name)| ReleaseRef::new(cluster.clone(), namespace, name));
        let key = SharedString::from(item_key(release.as_ref()));
        match workspace.read(cx).find_item_by_key(&key, cx) {
            Some(item) => workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx)),
            None => {
                let page = UpgradeDeps {
                    backend: deps.backend.clone(),
                    cluster: cluster.clone(),
                    cluster_label: label,
                };
                let view = cx.new(|cx| HelmUpgradeView::new(page, release, window, cx));
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
