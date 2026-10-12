//! The page over a scripted store: every kind counted, the breakdown, the history, namespaces,
//! and a page nobody looks at holding nothing.

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::{daemonset, deployment, job, node, pod, resource, statefulset};
use serde_json::json;

use super::{Fixture, cluster, no_grace};
use crate::pulses::{HISTORY_INTERVAL, Level, PULSE_SAMPLES};

/// A kind's `(total, rated, healthy)` as the store counted it. The status buckets are the
/// store's own business (E12-S07), so the assertions here compare the three the page reads.
fn tally(fx: &mut Fixture, id: &str) -> Option<(usize, usize, usize)> {
    fx.count_of(id)
        .and_then(|s| s.count())
        .map(|c| (c.total, c.rated, c.healthy))
}

/// A cluster with something of every kind of the page.
fn seed_everything(ports: &oxikube_testkit::FakeClusterPorts) {
    let r = &ports.resources;
    r.insert(node().name("n1").ready().build());
    r.insert(node().name("n2").ready().build());
    r.insert(node().name("n3").not_ready().build());
    r.insert(
        resource("v1", "Namespace")
            .cluster_scoped()
            .name("a")
            .build(),
    );
    r.insert(
        resource("v1", "Namespace")
            .cluster_scoped()
            .name("b")
            .build(),
    );
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
    r.insert(
        statefulset()
            .namespace("a")
            .name("db")
            .replicas(1)
            .ready(1)
            .build(),
    );
    r.insert(daemonset().namespace("a").name("agent").build());
    r.insert(job().namespace("a").name("migrate").failed().build());
    r.insert(job().namespace("a").name("seed").complete().build());
    r.insert(
        resource("batch/v1", "CronJob")
            .namespace("a")
            .name("nightly")
            .field("spec", json!({"schedule": "0 0 * * *", "suspend": true}))
            .build(),
    );
    r.insert(
        resource("v1", "PersistentVolume")
            .cluster_scoped()
            .name("pv1")
            .build(),
    );
    r.insert(
        resource("v1", "PersistentVolumeClaim")
            .namespace("a")
            .name("data")
            .build(),
    );
    r.insert(
        resource("autoscaling/v2", "HorizontalPodAutoscaler")
            .namespace("a")
            .name("web")
            .build(),
    );
    r.insert(
        resource("networking.k8s.io/v1", "Ingress")
            .namespace("a")
            .name("web")
            .build(),
    );
    r.insert(
        resource("networking.k8s.io/v1", "NetworkPolicy")
            .namespace("a")
            .name("deny")
            .build(),
    );
    r.insert(
        resource("v1", "ServiceAccount")
            .namespace("a")
            .name("default")
            .build(),
    );
}

#[gpui::test]
fn every_kind_of_the_story_is_counted_from_the_store(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), seed_everything);
    fx.tick();

    assert_eq!(tally(&mut fx, "nodes"), Some((3, 3, 2)), "n3 is not ready");
    assert_eq!(
        fx.count_of("namespaces")
            .and_then(|s| s.count())
            .map(|c| c.total),
        Some(2)
    );
    assert_eq!(tally(&mut fx, "pods"), Some((3, 3, 2)));
    assert_eq!(tally(&mut fx, "deployments"), Some((2, 2, 1)));
    assert_eq!(tally(&mut fx, "statefulsets"), Some((1, 1, 1)));
    assert!(
        fx.count_of("daemonsets")
            .is_some_and(|s| s.count().is_some())
    );
    assert_eq!(tally(&mut fx, "jobs"), Some((2, 2, 1)));
    assert_eq!(tally(&mut fx, "cronjobs"), Some((1, 1, 0)), "suspended");
    for (id, total) in [
        ("persistentvolumes", 1),
        ("persistentvolumeclaims", 1),
        ("horizontalpodautoscalers", 1),
        ("ingresses", 1),
        ("networkpolicies", 1),
        ("serviceaccounts", 1),
    ] {
        let count = fx.count_of(id).and_then(|s| s.count());
        assert_eq!(count.map(|c| c.total), Some(total), "{id}");
    }
}

#[gpui::test]
fn the_faces_carry_the_breakdown_and_the_level(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), seed_everything);
    fx.tick();
    let (pods, nodes, namespaces) = fx.read(|p| {
        let face = |id: &str| p.model().tile(id).expect(id).face().clone();
        (face("pods"), face("nodes"), face("namespaces"))
    });
    assert_eq!(pods.value, "3");
    assert_eq!(pods.caption, "2 healthy · 1 unhealthy");
    assert_eq!(pods.level, Level::Warn);
    assert_eq!(nodes.caption, "2 ready · 1 not ready");
    assert_eq!(namespaces.value, "2");
    assert_eq!(namespaces.gauge, None, "namespaces have no health rule");
}

#[gpui::test]
fn the_page_draws_sixteen_tiles(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), seed_everything);
    fx.tick();
    fx.draw();
    for spec in crate::pulses::tile_specs() {
        let selector: &'static str = Box::leak(format!("pulse-tile-{}", spec.id).into_boxed_str());
        assert!(
            fx.vcx.debug_bounds(selector).is_some(),
            "{selector} is drawn"
        );
    }
}

#[gpui::test]
fn tiles_keep_their_size_when_the_numbers_change(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |_| {});
    fx.tick();
    fx.draw();
    let before = fx.vcx.debug_bounds("pulse-tile-pods").expect("pods tile");
    for i in 0..120 {
        fx.ports
            .resources
            .insert(pod().namespace("a").name(format!("p{i}")).running().build());
    }
    fx.ports
        .resources
        .insert(job().namespace("a").name("big").failed().build());
    fx.ticks(3);
    fx.draw();
    let after = fx.vcx.debug_bounds("pulse-tile-pods").expect("pods tile");
    assert_eq!(before.size, after.size, "no reflow");
    assert_eq!(before.origin, after.origin);
}

#[gpui::test]
fn the_history_takes_a_sample_every_interval_and_is_capped(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        ports
            .resources
            .insert(pod().namespace("a").name("p1").running().build());
    });
    let every = HISTORY_INTERVAL.as_secs() as usize;
    fx.ticks(every);
    let first = fx.read(|p| p.model().tile("pods").expect("pods").ring.len());
    assert!(first >= 1, "a sample after the first interval");
    fx.ticks(every * (PULSE_SAMPLES + 5));
    let (len, ascending) = fx.read(|p| {
        let ring = &p.model().tile("pods").expect("pods").ring;
        let points: Vec<_> = ring.iter().collect();
        (ring.len(), points.windows(2).all(|w| w[0].t < w[1].t))
    });
    assert_eq!(len, PULSE_SAMPLES, "capped");
    assert!(ascending);
}

#[gpui::test]
fn a_new_namespace_selection_recounts_the_namespaced_kinds(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), seed_everything);
    fx.tick();
    assert_eq!(
        fx.count_of("pods").and_then(|s| s.count()).map(|c| c.total),
        Some(3)
    );
    let sessions = fx.sessions.clone();
    sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("b"))
        .expect("selected");
    fx.ticks(2);
    assert_eq!(
        fx.count_of("pods").and_then(|s| s.count()).map(|c| c.total),
        Some(1),
        "only namespace b"
    );
    assert_eq!(
        fx.count_of("nodes")
            .and_then(|s| s.count())
            .map(|c| c.total),
        Some(3),
        "cluster-scoped kinds ignore the selection"
    );
}

#[gpui::test]
fn a_forbidden_kind_says_no_access_and_not_zero(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        ports
            .resources
            .script()
            .watch
            .push_err(OxiError::forbidden("nodes is forbidden"));
    });
    fx.ticks(2);
    let denied = fx
        .count_of("nodes")
        .or_else(|| fx.count_of("namespaces"))
        .expect("a state");
    assert!(
        fx.read(|p| p.model().tiles().iter().any(|t| t.count.is_no_access())),
        "{denied:?}"
    );
    let face = fx.read(|p| {
        p.model()
            .tiles()
            .iter()
            .find(|t| t.count.is_no_access())
            .map(|t| t.face().clone())
            .expect("a denied tile")
    });
    assert_eq!(face.value, "no access");
}

#[gpui::test]
fn a_page_in_a_background_tab_holds_nothing(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), seed_everything);
    fx.tick();
    assert!(
        fx.read(|p| p.lease.is_some() && p.interest.is_some()),
        "shown: it holds both"
    );

    let page = fx.page.clone();
    fx.vcx.update(|window, cx| {
        page.update(cx, |page, cx| {
            oxikube_workspace::Item::set_active(page, false, window, cx);
        });
    });
    assert!(
        fx.read(|p| p.lease.is_none() && p.interest.is_none()),
        "hidden: no lease, no metrics interest"
    );
    fx.ticks(5);
    assert!(fx.read(|p| p.lease.is_none()), "and it stays idle");

    fx.vcx.update(|window, cx| {
        page.update(cx, |page, cx| {
            oxikube_workspace::Item::set_active(page, true, window, cx);
        });
    });
    fx.tick();
    assert!(
        fx.read(|p| p.lease.is_some() && p.interest.is_some()),
        "shown again"
    );
    assert_eq!(tally(&mut fx, "pods"), Some((3, 3, 2)));
}

#[gpui::test]
fn closing_the_page_releases_the_feeds(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), seed_everything);
    fx.tick();
    fx.keys("escape");
    let remaining = fx.ws.read_with(&fx.vcx, |ws, _| ws.items().count());
    assert_eq!(remaining, 0, "escape closed the page");
    assert!(fx.read(|p| p.lease.is_none() && p.interest.is_none() && p.store.is_none()));
}

#[gpui::test]
fn a_cluster_that_is_not_connected_says_so(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, no_grace());
    fx.tick();
    let sessions = fx.sessions.clone();
    sessions.disconnect(&cluster()).expect("disconnect");
    fx.ticks(2);
    fx.draw();
    assert!(fx.vcx.debug_bounds("pulses-not-connected").is_some());
    assert!(fx.read(|p| p.lease.is_none()));
}
