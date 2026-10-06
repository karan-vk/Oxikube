//! The window's chrome around the cluster tabs, on the real init path: the hotbar (E06-S04) as the
//! workspace strip, the active cluster's status bar badge (E06-S09) following the displayed tab,
//! and session restore (E06-S11) reopening the saved cluster after the first frame.

use futures::executor::block_on;
use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::session::restore::{ClusterTabsStore, SavedTabs};
use oxikube_catalog_ui::CatalogView;
use oxikube_settings::SettingsStore;
use oxikube_testkit::TestPorts;
use oxikube_workspace::persistence::MAIN_WINDOW_ID;

use super::App;

#[gpui::test]
fn the_hotbar_is_the_window_strip_and_marks_the_displayed_cluster(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(app.drawn("hotbar"), "the hotbar is mounted as the strip");

    app.press("enter");
    let name = TestPorts::CONTEXT;
    assert!(
        app.drawn(&format!("hotbar-tile-{name}")),
        "the connected cluster has a tile"
    );
    assert!(
        app.drawn(&format!("hotbar-active-{name}")),
        "its tile is marked as the displayed one"
    );
}

#[gpui::test]
fn the_status_bar_badge_follows_the_displayed_cluster(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert!(
        !app.drawn("status-cluster"),
        "no badge while the catalog is displayed"
    );

    app.press("enter");
    assert!(
        app.drawn("status-cluster"),
        "the badge shows the connected cluster's tab"
    );

    // Back to the catalog: the badge follows the active tab away again.
    let catalog = app.read(|ws, _| ws.items_of_type::<CatalogView>()[0].entity_id());
    let workspace = app.workspace();
    app.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.activate_item(catalog, true, window, cx));
    });
    app.vcx.run_until_parked();
    assert!(
        !app.drawn("status-cluster"),
        "no badge once the catalog is displayed again"
    );
}

#[gpui::test]
fn session_restore_reopens_and_connects_the_saved_cluster(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    let store = ClusterTabsStore::new(ports.state.clone(), MAIN_WINDOW_ID).expect("the store");
    let saved = SavedTabs::new(vec![cluster.clone()], Some(cluster.clone()))
        .with_titles([(cluster.clone(), TestPorts::CONTEXT.to_owned())]);
    block_on(store.save(&saved)).expect("the last session is saved");

    let mut app = App::start_with(cx, ports, |cx| {
        cx.update_global::<SettingsStore, _>(|store, _| {
            store
                .set_user_settings(r#"{ "session": { "restore": true } }"#)
                .expect("valid settings");
        });
    });
    assert!(
        app.cluster_tabs().is_empty(),
        "nothing is restored before the first frame"
    );

    // Tests have no platform frame loop: draw the first frame and deliver it.
    app.vcx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.simulate_next_frame(cx);
    });
    app.vcx.run_until_parked();

    assert_eq!(app.cluster_tabs().len(), 1, "the saved tab is back");
    assert_eq!(
        app.ports.connector.live_connections(&cluster),
        1,
        "the displayed cluster connects (`restore_connect: active`)"
    );
}
