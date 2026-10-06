//! States and diagnostics (E07-S10) in a window: loading, empty, filtered-empty, forbidden,
//! unauthorized and error, the stale badge, Retry through the command bus, and the API server's
//! warnings as toasts, once each.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Entity, TestAppContext};
use oxikube_app::store::StoreFilter;
use oxikube_domain::command::Command;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{ResourceCall, Timeline};

use super::fixture::{Fixture, cluster};
use super::p;
use crate::table::states::{Stale, copy};
use crate::table::{ResourceTable, ResourceTableEvent, TableState};

fn draw(f: &mut Fixture) {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

fn state(f: &mut Fixture, table: &Entity<ResourceTable>) -> TableState {
    f.vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.table_state()))
}

fn title(f: &mut Fixture, table: &Entity<ResourceTable>) -> String {
    f.vcx.update(|_, cx| {
        table
            .read(cx)
            .read_rows(cx, |d| copy(&d.table_state(), d.labels()).title)
    })
}

fn watches(f: &Fixture) -> usize {
    f.ports()
        .resources
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, ResourceCall::Watch { .. }))
        .count()
}

/// Clicks the element `selector` names (after a draw).
fn click(f: &mut Fixture, selector: &'static str) {
    draw(f);
    let at = f
        .vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was not laid out"))
        .center();
    f.vcx.simulate_click(at, gpui::Modifiers::none());
    f.settle();
}

fn shown(f: &mut Fixture, selector: &'static str) -> bool {
    draw(f);
    f.vcx.debug_bounds(selector).is_some()
}

fn restarted(names: &[&str], rv: &str) -> DeltaBatch<oxikube_domain::Resource> {
    DeltaBatch::from_deltas(vec![Delta::Restarted(
        names.iter().map(|n| p("x", n, rv)).collect(),
    )])
}

#[gpui::test]
fn loading_shows_a_skeleton_and_a_spinner_until_the_list_arrives(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let ports = f.ports();
    ports.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(Duration::from_secs(5), restarted(&["a", "b"], "1"))
            .keep_open(),
    );
    f.connect_with([]);
    let table = f.open_pods();
    assert_eq!(state(&mut f, &table), TableState::Loading);
    assert_eq!(title(&mut f, &table), "Loading pods…");
    assert!(shown(&mut f, "resource-table-skeleton"));
    assert!(shown(&mut f, "resource-table-spinner"));
    assert!(
        !shown(&mut f, "resource-table-retry"),
        "nothing to retry yet"
    );

    ports.resources.clock().advance(Duration::from_secs(5));
    f.settle();
    assert_eq!(f.names(&table), ["a", "b"]);
    assert_eq!(state(&mut f, &table), TableState::Rows { stale: None });
    assert!(!shown(&mut f, "resource-table-state"));
}

#[gpui::test]
fn empty_is_not_loading_and_names_the_scope(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([]);
    let table = f.open_pods();
    assert_eq!(state(&mut f, &table), TableState::Empty);
    assert_eq!(title(&mut f, &table), "No pods in all namespaces");
    assert!(!shown(&mut f, "resource-table-skeleton"));
    assert!(!shown(&mut f, "resource-table-spinner"));
    assert!(
        !shown(&mut f, "resource-table-retry"),
        "an empty list is not a failure"
    );
}

#[gpui::test]
fn filtered_empty_names_the_filter_and_one_click_clears_it(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "db-0", "1")]);
    let table = f.open_pods();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let _subscription = f.vcx.update(|_, cx| {
        cx.subscribe(&table, move |_, event: &ResourceTableEvent, _| {
            sink.borrow_mut().push(event.clone());
        })
    });
    f.update(&table, |t, cx| {
        t.set_filter(StoreFilter::text("zzz"), Some("zzz".into()), cx)
    });
    assert_eq!(f.names(&table), Vec::<String>::new());
    assert_eq!(
        state(&mut f, &table),
        TableState::FilteredEmpty {
            filter: "zzz".into()
        }
    );
    assert_eq!(title(&mut f, &table), "No pods match “zzz”");
    assert!(shown(&mut f, "resource-table-clear-filter"));

    click(&mut f, "resource-table-clear-filter");
    assert_eq!(f.names(&table), ["db-0", "web-0"]);
    assert_eq!(state(&mut f, &table), TableState::Rows { stale: None });
    assert!(events.borrow().contains(&ResourceTableEvent::FilterCleared));
}

#[gpui::test]
fn forbidden_names_the_verb_and_resource_and_retry_recovers(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let ports = f.ports();
    ports.resources.script().watch.push_err(OxiError::forbidden(
        "pods is forbidden: User \"me\" cannot list resource \"pods\"",
    ));
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    assert!(matches!(
        state(&mut f, &table),
        TableState::Forbidden { .. }
    ));
    assert_eq!(title(&mut f, &table), "Not allowed to list pods");
    assert!(shown(&mut f, "resource-table-retry"));
    assert!(shown(&mut f, "resource-table-state-summary"));
    assert_eq!(watches(&f), 1);

    // Retry is a command: the palette, the key and an agent send the same one.
    f.dispatcher.clear();
    click(&mut f, "resource-table-retry");
    assert!(matches!(
        f.dispatcher.sent().as_slice(),
        [Command::ResourceRetryFeed { cluster: c, gvk }] if *c == cluster() && gvk.kind.as_ref() == "Pod"
    ));
    assert_eq!(watches(&f), 2, "the click re-subscribed the feed");
    assert_eq!(f.names(&table), ["web-0"], "the access came back");
    assert_eq!(state(&mut f, &table), TableState::Rows { stale: None });
}

#[gpui::test]
fn expired_credentials_read_as_auth_not_as_an_unknown_resource(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.ports().resources.script().watch.push_err(OxiError::auth(
        "the server could not find the requested resource",
        false,
    ));
    f.connect_with([]);
    let table = f.open_pods();
    assert!(matches!(
        state(&mut f, &table),
        TableState::Unauthorized { .. }
    ));
    assert_eq!(title(&mut f, &table), "Your credentials were rejected");
    assert!(shown(&mut f, "resource-table-retry"));
}

#[gpui::test]
fn a_failure_shows_a_short_message_and_the_detail_on_expand(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let long = format!("the server is unhappy\n{}", "stack frame ".repeat(30));
    f.ports()
        .resources
        .script()
        .watch
        .push_err(OxiError::not_found(long));
    f.connect_with([]);
    let table = f.open_pods();
    assert!(matches!(
        state(&mut f, &table),
        TableState::Failed {
            kind: ErrorKind::NotFound,
            ..
        }
    ));
    assert_eq!(title(&mut f, &table), "Cannot list pods");
    assert!(shown(&mut f, "resource-table-state-summary"));
    assert!(!shown(&mut f, "resource-table-state-detail"), "collapsed");
    click(&mut f, "resource-table-details-toggle");
    assert!(shown(&mut f, "resource-table-state-detail"));
    click(&mut f, "resource-table-details-toggle");
    assert!(!shown(&mut f, "resource-table-state-detail"));
}

#[gpui::test]
fn a_dropped_first_watch_is_reconnecting_and_retry_skips_the_wait(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.ports()
        .resources
        .script()
        .watch
        .push_err(OxiError::network("connection refused"));
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    assert!(matches!(
        state(&mut f, &table),
        TableState::Reconnecting { .. }
    ));
    assert!(shown(&mut f, "resource-table-spinner"));
    click(&mut f, "resource-table-retry");
    assert_eq!(f.names(&table), ["web-0"]);
}

#[gpui::test]
fn retry_keeps_the_old_rows_with_a_stale_badge_until_new_data(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let ports = f.ports();
    ports.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(Duration::ZERO, restarted(&["a", "b"], "1"))
            .err_at(
                Duration::from_secs(1),
                OxiError::forbidden("no longer allowed"),
            ),
    );
    // What the second attempt lists, a few seconds later.
    ports.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(Duration::from_secs(10), restarted(&["a", "b", "c"], "2"))
            .keep_open(),
    );
    f.connect_with([]);
    let table = f.open_pods();
    assert_eq!(f.names(&table), ["a", "b"]);
    assert!(!shown(&mut f, "resource-table-stale"));

    ports.resources.clock().advance(Duration::from_secs(1));
    f.settle();
    assert_eq!(f.names(&table), ["a", "b"], "the rows stay under the error");
    assert_eq!(
        state(&mut f, &table),
        TableState::Rows {
            stale: Some(Stale::Forbidden)
        }
    );
    assert!(shown(&mut f, "resource-table-stale"));

    click(&mut f, "resource-table-stale-retry");
    assert_eq!(watches(&f), 2);
    assert_eq!(
        f.names(&table),
        ["a", "b"],
        "still the old rows while it lists"
    );
    assert_eq!(
        state(&mut f, &table),
        TableState::Rows {
            stale: Some(Stale::Refreshing)
        }
    );
    assert!(shown(&mut f, "resource-table-stale"));

    ports.resources.clock().advance(Duration::from_secs(10));
    f.settle();
    assert_eq!(f.names(&table), ["a", "b", "c"]);
    assert_eq!(state(&mut f, &table), TableState::Rows { stale: None });
    assert!(
        !shown(&mut f, "resource-table-stale"),
        "the badge went with the staleness"
    );
}

fn toasts(f: &mut Fixture) -> Vec<String> {
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    f.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .toast_layer()
            .read(cx)
            .visible()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

#[gpui::test]
fn an_api_warning_is_one_toast_per_distinct_text(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let first = f.open_pods();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let _subscription = f.vcx.update(|_, cx| {
        cx.subscribe(&first, move |_, event: &ResourceTableEvent, _| {
            if let ResourceTableEvent::ApiWarning(w) = event {
                sink.borrow_mut().push(w.text.clone());
            }
        })
    });
    let warnings = f.ports().warnings;
    warnings.push_text("v1 Endpoints is deprecated in v1.33+");
    f.settle();
    warnings.push_text("v1 Endpoints is deprecated in v1.33+");
    f.settle();
    assert_eq!(
        toasts(&mut f),
        ["v1 Endpoints is deprecated in v1.33+"],
        "the same warning twice is one toast"
    );
    assert_eq!(events.borrow().len(), 1);

    warnings.push_text("unknown field \"spec.nope\"");
    f.settle();
    assert_eq!(
        toasts(&mut f),
        [
            "v1 Endpoints is deprecated in v1.33+",
            "unknown field \"spec.nope\""
        ],
        "different text is another toast"
    );
}

#[gpui::test]
fn two_tables_of_one_cluster_show_a_warning_once(cx: &mut TestAppContext) {
    use oxikube_domain::ids::Gvk;
    use oxikube_domain::kinds::{ResourceKind, VerbSet};

    let mut f = Fixture::new(cx);
    let services = ResourceKind {
        gvk: Gvk::new("", "v1", "Service"),
        preferred: true,
        plural: "services".into(),
        singular: "service".into(),
        short_names: vec!["svc".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced: true,
    };
    f.connect_with([]);
    f.ports()
        .discovery
        .set_kinds([super::pods_kind(), services.clone()]);
    f.open_pods();
    f.open(services);
    f.ports().warnings.push_text("shared deprecation");
    f.settle();
    assert_eq!(toasts(&mut f), ["shared deprecation"]);
}
