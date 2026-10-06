//! The count badges over a `ResourceStores` on fake feeds: they come from the store, follow it,
//! say "no access" for a forbidden kind, follow the namespace selection, and redraw at the
//! timer's cadence however many events arrive.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::{CountState, KindCount};
use oxikube_domain::OxiError;
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{Timeline, pod};

use super::{Fixture, can_list};
use crate::sidebar::Row;

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
    })
}

/// Lets the panel's timer fire once.
fn tick(fx: &mut Fixture) {
    fx.vcx.executor().advance_clock(Duration::from_secs(1));
    fx.vcx.run_until_parked();
}

fn count(fx: &mut Fixture, group: &'static str, plural: &'static str) -> Option<CountState> {
    let panel = fx.panel.clone();
    fx.vcx
        .update(|_, cx| panel.read(cx).count_of(group, plural).cloned())
}

fn entry_count(fx: &mut Fixture, id: &str) -> Option<CountState> {
    fx.rows().into_iter().find_map(|row| match row {
        Row::Entry(e) if &*e.id == id => e.count,
        _ => None,
    })
}

fn pods(items: &[(&str, &str, bool)]) -> Vec<oxikube_domain::Resource> {
    items
        .iter()
        .map(|(ns, name, running)| {
            let p = pod().namespace(*ns).name(*name);
            if *running { p.running() } else { p.pending() }.build()
        })
        .collect()
}

#[gpui::test]
fn the_sidebar_shows_counts_from_the_store(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_counted(cx, AccessRules::all_access());
    for p in pods(&[("a", "p1", true), ("a", "p2", true), ("b", "p3", false)]) {
        fx.ports.resources.insert(p);
    }
    fx.connect();
    tick(&mut fx);
    assert_eq!(count(&mut fx, "", "pods"), Some(counted(3, 3, 2)));
    assert_eq!(
        entry_count(&mut fx, "workloads/pods"),
        Some(counted(3, 3, 2)),
        "the row carries it"
    );
    // The eagerly counted kinds are open; nothing else was started for the sake of a number.
    assert_eq!(
        count(&mut fx, "apps", "deployments"),
        Some(counted(0, 0, 0))
    );
    assert_eq!(count(&mut fx, "", "configmaps"), None);
    assert_eq!(entry_count(&mut fx, "config/configmaps"), None);
    assert_eq!(
        fx.ports.resources.live_watches(),
        4,
        "pods, nodes, namespaces, deployments"
    );
    let leased = fx.panel.read_with(&fx.vcx, |p, _| p.counts_lease_len());
    assert_eq!(leased, 4);
}

#[gpui::test]
fn a_kind_gets_a_badge_when_something_else_opens_its_feed(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_counted(cx, AccessRules::all_access());
    fx.connect();
    tick(&mut fx);
    assert_eq!(count(&mut fx, "", "configmaps"), None);

    let session = fx.sessions.get(&fx.cluster).expect("a session");
    let store = fx
        .stores
        .as_ref()
        .and_then(|s| s.for_session(&session))
        .expect("a store");
    // A table opens the feed: the badge reads it, and starts none of its own.
    let table = store.subscribe(oxikube_app::StoreQuery::new(
        Gvk::new("", "v1", "ConfigMap"),
        WatchScope::Cluster,
    ));
    fx.vcx.run_until_parked();
    let before = fx.ports.resources.live_watches();
    tick(&mut fx);
    assert_eq!(count(&mut fx, "", "configmaps"), Some(counted(0, 0, 0)));
    assert_eq!(
        fx.ports.resources.live_watches(),
        before,
        "counting started no feed"
    );
    drop(table);
}

#[gpui::test]
fn a_forbidden_kind_says_no_access_not_zero(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_counted(cx, can_list(&[("", "pods")]));
    fx.ports
        .resources
        .script()
        .watch
        .push_err(OxiError::forbidden("pods is forbidden: User cannot list"));
    fx.select_namespace("dev");
    fx.connect();
    tick(&mut fx);
    let state = count(&mut fx, "", "pods").expect("an answer");
    assert!(state.is_no_access(), "{state:?}");
    assert!(matches!(
        entry_count(&mut fx, "workloads/pods"),
        Some(CountState::NoAccess { message }) if message.contains("forbidden")
    ));
}

#[gpui::test]
fn counts_follow_the_namespace_selection(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_counted(cx, AccessRules::all_access());
    for p in pods(&[("a", "p1", true), ("b", "p2", false), ("b", "p3", true)]) {
        fx.ports.resources.insert(p);
    }
    fx.connect();
    tick(&mut fx);
    assert_eq!(count(&mut fx, "", "pods"), Some(counted(3, 3, 2)));
    fx.select_namespace("b");
    tick(&mut fx);
    assert_eq!(count(&mut fx, "", "pods"), Some(counted(2, 2, 1)));
}

#[gpui::test]
fn disconnecting_clears_the_badges_and_the_lease(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_counted(cx, AccessRules::all_access());
    fx.connect();
    tick(&mut fx);
    assert!(count(&mut fx, "", "pods").is_some());
    fx.disconnect();
    tick(&mut fx);
    assert_eq!(count(&mut fx, "", "pods"), None);
    assert_eq!(entry_count(&mut fx, "workloads/pods"), None);
    let leased = fx.panel.read_with(&fx.vcx, |p, _| p.counts_lease_len());
    assert_eq!(leased, 0);
}

#[gpui::test]
fn a_thousand_deltas_in_a_second_redraw_the_sidebar_a_handful_of_times(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_counted(cx, can_list(&[("", "pods")]));
    fx.select_namespace("dev");
    // One pod added every millisecond for a second.
    let timeline = (0..1000u64).fold(Timeline::new(), |t, i| {
        let object = pod()
            .namespace("dev")
            .name(format!("p{i}"))
            .running()
            .build();
        t.ok_at(
            Duration::from_millis(i),
            DeltaBatch::from_deltas(vec![Delta::Applied(object)]),
        )
    });
    fx.ports
        .resources
        .script()
        .watch
        .push_ok(timeline.keep_open());
    fx.connect();
    tick(&mut fx);

    let renders = Rc::new(Cell::new(0usize));
    let seen = renders.clone();
    let panel = fx.panel.clone();
    let _observe = fx
        .vcx
        .update(|_, cx| cx.observe(&panel, move |_, _| seen.set(seen.get() + 1)));
    fx.ports.resources.clock().advance(Duration::from_secs(1));
    fx.vcx.run_until_parked();
    tick(&mut fx);
    assert_eq!(count(&mut fx, "", "pods"), Some(counted(1000, 1000, 1000)));
    assert!(
        renders.get() <= 3,
        "1000 deltas redrew the sidebar {} times",
        renders.get()
    );
}
