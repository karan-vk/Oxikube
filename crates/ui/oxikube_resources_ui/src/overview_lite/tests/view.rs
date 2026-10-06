//! The overview with a scripted store: counts, health, no access, budget, namespaces, and the
//! click that opens a kind's list.

use std::sync::Arc;

use gpui::{Bounds, Pixels, Point, TestAppContext, point};
use oxikube_app::store::{MaxFeeds, StoreOptions};
use oxikube_app::{CountState, KindCount};
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::{daemonset, deployment, job, pod};

use super::{Fixture, cluster, no_grace};

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
    })
}

fn center(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    )
}

fn draw(fx: &mut Fixture) {
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn tiles_show_total_and_healthy_from_the_store(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        let r = &ports.resources;
        r.insert(pod().namespace("a").name("p1").running().build());
        r.insert(pod().namespace("a").name("p2").pending().build());
        r.insert(pod().namespace("b").name("p3").succeeded().build());
        r.insert(
            deployment()
                .namespace("a")
                .name("web")
                .replicas(3)
                .ready(3)
                .build(),
        );
        r.insert(
            deployment()
                .namespace("a")
                .name("api")
                .replicas(2)
                .ready(0)
                .build(),
        );
        r.insert(daemonset().namespace("a").name("agent").build());
        r.insert(job().namespace("a").name("migrate").failed().build());
        r.insert(job().namespace("a").name("seed").complete().build());
    });
    fx.tick();

    assert_eq!(fx.state("pods"), Some(counted(3, 3, 2)));
    assert_eq!(fx.state("deployments"), Some(counted(2, 2, 1)));
    assert_eq!(fx.state("jobs"), Some(counted(2, 2, 1)));
    assert_eq!(fx.state("statefulsets"), Some(counted(0, 0, 0)));
    assert_eq!(fx.state("cronjobs"), Some(counted(0, 0, 0)));
    assert!(fx.state("daemonsets").is_some_and(|s| s.count().is_some()));
}

#[gpui::test]
fn the_tiles_hold_exactly_seven_feeds_and_release_them_on_close(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, no_grace());
    fx.tick();
    assert_eq!(
        fx.ports.resources.live_watches(),
        7,
        "one feed per tile kind"
    );
    let (ws, id) = (fx.ws.clone(), fx.overview.entity_id());
    fx.vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.close_item(id, window, cx));
    });
    fx.vcx.run_until_parked();
    assert_eq!(
        fx.ports.resources.live_watches(),
        0,
        "closing the item stops its feeds"
    );
}

#[gpui::test]
fn a_forbidden_kind_says_no_access_not_zero(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        ports
            .resources
            .script()
            .watch
            .push_err(OxiError::forbidden("a kind is forbidden"));
    });
    fx.tick();
    let ids = [
        "deployments",
        "statefulsets",
        "daemonsets",
        "replicasets",
        "jobs",
        "cronjobs",
        "pods",
    ];
    let denied = ids
        .into_iter()
        .filter(|id| fx.state(id).is_some_and(|s| s.is_no_access()))
        .count();
    assert_eq!(denied, 1, "the one kind whose feed was refused");
    let counted = ids
        .into_iter()
        .filter(|id| fx.state(id).is_some_and(|s| s.count().is_some()))
        .count();
    assert_eq!(counted, 6, "the others are counted");
}

#[gpui::test]
fn kinds_over_the_budget_degrade_to_a_dash_and_start_no_feed(cx: &mut TestAppContext) {
    let options = StoreOptions {
        budget: Arc::new(MaxFeeds::new(3)),
        ..no_grace()
    };
    let mut fx = Fixture::open(cx, options);
    fx.tick();
    let over = [
        "deployments",
        "statefulsets",
        "daemonsets",
        "replicasets",
        "jobs",
        "cronjobs",
        "pods",
    ]
    .into_iter()
    .filter(|id| matches!(fx.state(id), Some(CountState::OverBudget { .. })))
    .count();
    assert!(over >= 3, "{over} tiles were refused");
    assert!(
        fx.ports.resources.live_watches() <= 4,
        "pods may use the headroom of a priority kind"
    );
}

#[gpui::test]
fn the_namespace_selection_scopes_the_counts(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        let r = &ports.resources;
        r.insert(pod().namespace("a").name("p1").running().build());
        r.insert(pod().namespace("b").name("p2").pending().build());
        r.insert(pod().namespace("b").name("p3").running().build());
    });
    fx.tick();
    assert_eq!(fx.state("pods"), Some(counted(3, 3, 2)));
    fx.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("b"))
        .expect("open session");
    fx.vcx.run_until_parked();
    fx.tick();
    assert_eq!(fx.state("pods"), Some(counted(2, 2, 1)));
}

#[gpui::test]
fn a_disconnected_cluster_shows_that_instead_of_zeros(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_disconnected(cx);
    draw(&mut fx);
    assert!(fx.vcx.debug_bounds("overview-not-connected").is_some());
    assert!(fx.vcx.debug_bounds("overview-tile-pods").is_none());
    assert_eq!(fx.state("pods"), Some(CountState::NotWatched));
    assert_eq!(fx.ports.resources.live_watches(), 0, "nothing started");
}

#[gpui::test]
fn clicking_a_tile_sends_the_open_list_command(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, no_grace());
    fx.tick();
    draw(&mut fx);
    let bounds = fx
        .vcx
        .debug_bounds("overview-tile-deployments")
        .expect("the tile is on screen");
    fx.vcx.simulate_click(center(bounds), Default::default());
    fx.vcx.run_until_parked();
    assert_eq!(
        fx.recorder.sent(),
        [Command::ResourceOpenList {
            cluster: cluster(),
            gvk: Gvk::new("apps", "v1", "Deployment"),
        }]
    );
    let bounds = fx
        .vcx
        .debug_bounds("overview-tile-pods")
        .expect("the pods tile is on screen");
    fx.vcx.simulate_click(center(bounds), Default::default());
    fx.vcx.run_until_parked();
    assert_eq!(fx.recorder.sent().len(), 2);
    assert!(matches!(
        fx.recorder.sent().last(),
        Some(Command::ResourceOpenList { gvk, .. }) if &*gvk.kind == "Pod"
    ));
}

#[gpui::test]
fn opening_the_overview_again_shows_the_open_one(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, no_grace());
    let deps = fx.deps.clone();
    let ws = fx.ws.clone();
    fx.vcx.update(|window, cx| {
        super::WorkloadsOverview::open(&ws, cluster(), deps, window, cx);
    });
    fx.vcx.run_until_parked();
    let count = fx.vcx.update(|_, cx| {
        ws.read(cx)
            .items_of_type::<super::WorkloadsOverview>()
            .len()
    });
    assert_eq!(count, 1, "one overview per cluster tab");
}
