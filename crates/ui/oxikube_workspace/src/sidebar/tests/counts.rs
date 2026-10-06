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
fn a_forbidden_single_kind_section_says_no_access_too(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_counted(cx, can_list(&[("", "nodes")]));
    fx.ports
        .resources
        .script()
        .watch
        .push_err(OxiError::forbidden("nodes is forbidden: User cannot list"));
    fx.connect();
    tick(&mut fx);
    let section = fx.rows().into_iter().find_map(|row| match row {
        Row::Section(s) if &*s.id == "nodes" => Some(s),
        _ => None,
    });
    assert!(
        matches!(
            section.and_then(|s| s.count),
            Some(CountState::NoAccess { message }) if message.contains("forbidden")
        ),
        "the collapsed Nodes row carries the state, not a bare dash"
    );
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

/// A kind in `group` at `version`, listable and watchable, as discovery serves it.
fn served(
    group: &str,
    kind: &str,
    plural: &str,
    namespaced: bool,
) -> oxikube_domain::kinds::ResourceKind {
    let mut kind = super::crd(group, kind, plural);
    kind.namespaced = namespaced;
    kind
}

#[gpui::test]
fn expanding_the_custom_resources_starts_no_feed_and_a_kind_is_counted_once_a_table_opens_it(
    cx: &mut TestAppContext,
) {
    let mut fx = Fixture::open_counted(cx, AccessRules::all_access());
    let kinds: Vec<_> = (0..40)
        .map(|i| {
            served(
                &format!("group{}.example.com", i % 8),
                &format!("Kind{i}"),
                &format!("kind{i}s"),
                i % 2 == 0,
            )
        })
        .collect();
    fx.ports.discovery.set_kinds(kinds);
    fx.connect();
    tick(&mut fx);
    let before = (
        fx.ports.resources.live_watches(),
        fx.ports.tables.live_feeds(),
    );
    assert_eq!(before.1, 0, "no table feed for a custom kind");

    // Open the section's every group: 8 groups, 40 kinds on screen.
    for group in 0..8 {
        assert!(
            fx.toggle(&format!("crd:group{group}.example.com")),
            "expand"
        );
    }
    tick(&mut fx);
    let rows = fx.row_ids();
    assert_eq!(
        rows.iter()
            .filter(|id| id.starts_with("crd:group") && id.contains('/'))
            .count(),
        40
    );
    assert_eq!(
        (
            fx.ports.resources.live_watches(),
            fx.ports.tables.live_feeds()
        ),
        before,
        "expanding the sidebar started no feed"
    );
    assert_eq!(
        fx.panel.read_with(&fx.vcx, |p, _| p.counts_lease_len()),
        4,
        "only the eager kinds"
    );
    // The group rows say how many kinds each has; the kinds have no number yet.
    let groups: Vec<_> = fx
        .rows()
        .into_iter()
        .filter_map(|row| match row {
            Row::Group(g) => Some(g.count),
            _ => None,
        })
        .collect();
    assert_eq!(groups, vec![Some(5); 8]);
    assert_eq!(count(&mut fx, "group0.example.com", "kind0s"), None);
    assert_eq!(entry_count(&mut fx, "crd:group0.example.com/kind0s"), None);

    // A table opens Kind0's feed (a namespaced kind, so it is read per scope): the badge reads
    // it and starts none of its own.
    let session = fx.sessions.get(&fx.cluster).expect("a session");
    let store = fx
        .stores
        .as_ref()
        .and_then(|s| s.for_session(&session))
        .expect("a store");
    fx.ports.tables.script().table_feed.push_ok(
        Timeline::immediate([oxikube_ports::TableBatch {
            columns: Some(std::sync::Arc::from(vec![oxikube_ports::TableColumn {
                name: "Name".into(),
                column_type: "string".into(),
                ..Default::default()
            }])),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
                oxikube_ports::TableRow {
                    cells: vec![serde_json::json!("one")],
                    meta: Some(oxikube_domain::ObjectMeta::named("one")),
                    object: None,
                },
                oxikube_ports::TableRow {
                    cells: vec![serde_json::json!("two")],
                    meta: Some(oxikube_domain::ObjectMeta::named("two")),
                    object: None,
                },
            ])]),
            source: oxikube_ports::TableSource::Server,
        }])
        .keep_open(),
    );
    let table = store.subscribe(oxikube_app::StoreQuery::new(
        Gvk::new("group0.example.com", "v1", "Kind0"),
        WatchScope::Cluster,
    ));
    fx.vcx.run_until_parked();
    let feeds = fx.ports.tables.live_feeds();
    tick(&mut fx);
    assert_eq!(
        count(&mut fx, "group0.example.com", "kind0s"),
        Some(counted(2, 0, 0))
    );
    assert_eq!(
        entry_count(&mut fx, "crd:group0.example.com/kind0s"),
        Some(counted(2, 0, 0))
    );
    assert_eq!(
        fx.ports.tables.live_feeds(),
        feeds,
        "counting started no feed"
    );
    drop(table);
}
