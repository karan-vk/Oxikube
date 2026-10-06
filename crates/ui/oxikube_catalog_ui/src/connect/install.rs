//! Putting the connect lifecycle into a cluster tab.

use gpui::{App, AppContext as _, Entity, Window};
use oxikube_app::ClusterSession;
use oxikube_workspace::cluster_tab::{ClusterTab, ConnectUi};

use super::banner::DegradedBanner;
use super::deps::ConnectDeps;
use super::view::ConnectView;

/// Gives `tab` its connect views: the body for a session that is connecting, needs credentials
/// or failed, and the banner for a degraded one. Returns the view, for tests and for hosts that
/// want to talk to it.
pub fn install(tab: &Entity<ClusterTab>, deps: &ConnectDeps, cx: &mut App) -> Entity<ConnectView> {
    let cluster = tab.read(cx).cluster().clone();
    let view = cx.new(|cx| ConnectView::new(deps.clone(), cluster, cx));
    let banner = cx.new(|cx| DegradedBanner::new(view.clone(), cx));
    let ui = ConnectUi {
        body: view.clone().into(),
        banner: banner.into(),
    };
    tab.update(cx, |tab, cx| tab.set_connect_ui(ui, cx));
    view
}

/// The [`TabSetup`](oxikube_workspace::cluster_tab::TabSetup) hook that installs the connect
/// views in every new cluster tab: pass it to `ClusterTabsDeps::with_setup`, or call it from
/// the setup that also adds the sidebar.
pub fn tab_setup(
    deps: ConnectDeps,
) -> impl Fn(&Entity<ClusterTab>, &ClusterSession, &mut Window, &mut App) + 'static {
    move |tab, _, _, cx| {
        install(tab, &deps, cx);
    }
}
