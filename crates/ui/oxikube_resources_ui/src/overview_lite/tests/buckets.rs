//! The bars in the view: the store's tallies reach the tiles, a transition moves one count, and a
//! click on a segment or a legend entry opens the kind's list filtered to that status.

use gpui::{Bounds, Pixels, Point, TestAppContext, point};
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::Bucket;
use oxikube_testkit::{daemonset, deployment, pod};

use super::{Fixture, cluster, no_grace};

fn center(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    )
}

/// Lets the coalesced redraw the numbers asked for come (the pacer's backstop delivers it when no
/// window presents frames), then draws it.
fn draw(fx: &mut Fixture) {
    fx.vcx
        .executor()
        .advance_clock(std::time::Duration::from_millis(50));
    fx.vcx.run_until_parked();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn seeded(cx: &mut TestAppContext) -> Fixture {
    Fixture::open_seeded(cx, no_grace(), |ports| {
        let r = &ports.resources;
        r.insert(pod().namespace("a").name("p1").running().build());
        r.insert(pod().namespace("a").name("p2").running().build());
        r.insert(pod().namespace("a").name("p3").pending().build());
        r.insert(pod().namespace("b").name("p4").failed().build());
        r.insert(pod().namespace("b").name("p5").succeeded().build());
        r.insert(
            daemonset()
                .namespace("a")
                .name("agent")
                .replicas(3)
                .ready(1)
                .build(),
        );
        r.insert(
            deployment()
                .namespace("a")
                .name("web")
                .replicas(2)
                .ready(2)
                .build(),
        );
    })
}

#[gpui::test]
fn the_tiles_carry_the_status_buckets_of_the_store(cx: &mut TestAppContext) {
    let mut fx = seeded(cx);
    fx.tick();
    assert_eq!(fx.bucket("pods", Bucket::Running), 2);
    assert_eq!(fx.bucket("pods", Bucket::Pending), 1);
    assert_eq!(fx.bucket("pods", Bucket::Failed), 1);
    assert_eq!(fx.bucket("pods", Bucket::Succeeded), 1);
    assert_eq!(fx.bucket("daemonsets", Bucket::NotReady), 1);
    assert_eq!(fx.bucket("deployments", Bucket::Ready), 1);
}

#[gpui::test]
fn clicking_a_segment_opens_the_list_filtered_to_its_status(cx: &mut TestAppContext) {
    let mut fx = seeded(cx);
    fx.tick();
    draw(&mut fx);
    let bounds = fx
        .vcx
        .debug_bounds("overview-bar-pods-pending")
        .expect("the pending segment is on screen");
    fx.vcx.simulate_click(center(bounds), Default::default());
    fx.vcx.run_until_parked();
    assert_eq!(
        fx.recorder.sent(),
        [
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: pod_gvk(),
            },
            Command::TableSetFilter {
                cluster: cluster(),
                gvk: pod_gvk(),
                text: "-s pending".to_owned(),
            },
        ],
        "one list, one filter: the click did not also reach the tile"
    );
}

#[gpui::test]
fn clicking_a_legend_entry_filters_like_its_segment(cx: &mut TestAppContext) {
    let mut fx = seeded(cx);
    fx.tick();
    draw(&mut fx);
    let bounds = fx
        .vcx
        .debug_bounds("overview-legend-daemonsets-not-ready")
        .expect("the legend entry is on screen");
    fx.vcx.simulate_click(center(bounds), Default::default());
    fx.vcx.run_until_parked();
    let sent = fx.recorder.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(matches!(
        &sent[1],
        Command::TableSetFilter { gvk, text, .. }
            if &*gvk.kind == "DaemonSet" && text == "-s not-ready"
    ));
}

#[gpui::test]
fn a_tile_without_health_buckets_draws_no_bar(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, no_grace());
    fx.tick();
    draw(&mut fx);
    assert!(fx.vcx.debug_bounds("overview-tile-pods").is_some());
    assert!(
        fx.vcx.debug_bounds("overview-bar-pods-running").is_none(),
        "an empty kind has no segments"
    );
}
