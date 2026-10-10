//! `table::SetFilter` (E11-S05): the `:` jump bar's `/filter` and `k=v` reach a table through the
//! command, whether the command or the table comes first.

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::error::{ErrorKind, OxiError};
use oxikube_domain::ids::Gvk;
use oxikube_ports::HealthSignal;
use oxikube_testkit::{ResourceCall, pod};

use super::fixture::{Fixture, cluster};
use crate::table::ResourceTable;

fn labelled(ns: &str, name: &str, app: &str) -> Resource {
    pod().namespace(ns).name(name).label("app", app).build()
}

fn fixtures() -> Vec<Resource> {
    vec![
        labelled("x", "web-1", "web"),
        labelled("x", "web-2", "web"),
        labelled("x", "db-0", "db"),
        labelled("y", "cache-web", "cache"),
        labelled("y", "api", "api"),
    ]
}

fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn set_filter(f: &mut Fixture, text: &str) {
    let dispatcher = f.deps.dispatcher.clone();
    let command = Command::TableSetFilter {
        cluster: cluster(),
        gvk: pods(),
        text: text.to_owned(),
    };
    f.vcx.update(|_, cx| dispatcher.dispatch(command, cx));
    f.settle();
}

fn open_list(f: &mut Fixture) {
    let dispatcher = f.deps.dispatcher.clone();
    let command = Command::ResourceOpenList {
        cluster: cluster(),
        gvk: pods(),
    };
    f.vcx.update(|_, cx| dispatcher.dispatch(command, cx));
    f.settle();
}

fn only_table(f: &mut Fixture) -> Entity<ResourceTable> {
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    let mut tables = f.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<ResourceTable>()
    });
    assert_eq!(tables.len(), 1, "one table");
    tables.remove(0)
}

fn bar_text(f: &mut Fixture, table: &Entity<ResourceTable>) -> String {
    f.vcx
        .update(|_, cx| table.read(cx).filter().read(cx).text().to_owned())
}

fn selectors(f: &Fixture) -> Vec<Option<String>> {
    f.ports()
        .resources
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            ResourceCall::Watch { options, .. } => Some(options.label_selector),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn a_filter_for_an_open_table_is_typed_into_its_bar(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    let table = f.open_pods();
    assert_eq!(f.names(&table).len(), 5);

    set_filter(&mut f, "web");
    assert_eq!(f.names(&table), ["web-1", "web-2", "cache-web"]);
    assert_eq!(
        bar_text(&mut f, &table),
        "web",
        "the user sees what filters"
    );

    set_filter(&mut f, "");
    assert_eq!(f.names(&table).len(), 5, "empty text clears the filter");
    assert_eq!(bar_text(&mut f, &table), "");
}

#[gpui::test]
fn a_filter_that_arrives_before_its_table_waits_for_it(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());

    // The jump bar sends the list and its filter together; the filter may be applied first.
    set_filter(&mut f, "^web");
    open_list(&mut f);
    let table = only_table(&mut f);
    assert_eq!(f.names(&table), ["web-1", "web-2"]);
    assert_eq!(bar_text(&mut f, &table), "^web");
}

#[gpui::test]
fn the_list_then_the_filter_gives_the_same_table(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    open_list(&mut f);
    set_filter(&mut f, "^web");
    let table = only_table(&mut f);
    assert_eq!(f.names(&table), ["web-1", "web-2"]);
}

#[gpui::test]
fn a_waiting_filter_is_used_once(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    set_filter(&mut f, "web");
    open_list(&mut f);
    let table = only_table(&mut f);
    set_filter(&mut f, "db");
    assert_eq!(f.names(&table), ["db-0"]);
    // Opening the same list again does not bring the first filter back.
    open_list(&mut f);
    assert_eq!(f.names(&table), ["db-0"]);
}

#[gpui::test]
fn a_selector_in_the_text_is_applied_by_the_server_with_the_name_filter(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    let table = f.open_pods();
    set_filter(&mut f, "web -l app=web");
    assert_eq!(f.names(&table), ["web-1", "web-2"]);
    assert_eq!(bar_text(&mut f, &table), "web -l app=web");
    assert_eq!(
        selectors(&f),
        [None, Some("app=web".to_owned())],
        "the feed was re-keyed with the selector"
    );
    // Only the selector.
    set_filter(&mut f, "-l app=db");
    assert_eq!(f.names(&table), ["db-0"]);
}

/// A cluster whose tab is open and whose session has no discovery (it dropped): `resource::OpenList`
/// cannot open the table, and the filter that came with it must not wait for a later one.
fn dropped_cluster(f: &mut Fixture) {
    f.connect_with(fixtures());
    // The connection is lost for good: the tab stays, with no discovery behind it.
    let lost = HealthSignal::failed(&OxiError::new(ErrorKind::Internal, "connection lost"));
    assert!(f.connector.report(&cluster(), lost));
    f.vcx.run_until_parked();
    let session = f.sessions.get(&cluster()).expect("session");
    assert!(session.discovery().is_none(), "no discovery while in Error");
}

fn reconnect_and_open(f: &mut Fixture) {
    block_on(f.sessions.reconnect(&cluster())).expect("reconnect");
    f.vcx.run_until_parked();
    let table = f.open_pods();
    // The user opens Pods from the sidebar: everything, not the stale filter.
    assert_eq!(f.names(&table).len(), 5);
    assert_eq!(bar_text(f, &table), "");
}

#[gpui::test]
fn a_filter_after_a_list_that_could_not_open_is_not_kept_for_a_later_open(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    dropped_cluster(&mut f);
    // `:pod /api` while the cluster is reconnecting.
    open_list(&mut f);
    set_filter(&mut f, "api");
    reconnect_and_open(&mut f);
}

#[gpui::test]
fn a_filter_before_a_list_that_could_not_open_is_not_kept_for_a_later_open(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    dropped_cluster(&mut f);
    set_filter(&mut f, "api");
    open_list(&mut f);
    reconnect_and_open(&mut f);
}

#[gpui::test]
fn a_failed_open_without_a_filter_does_not_drop_the_filter_of_a_later_jump(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    dropped_cluster(&mut f);
    // A bare `:pods` (or a sidebar click) that could not open: no filter followed to consume it.
    open_list(&mut f);
    block_on(f.sessions.reconnect(&cluster())).expect("reconnect");
    f.vcx.run_until_parked();
    // `:pods ^web` right after the reconnect: the discovery is new, so the list resolves
    // asynchronously and the filter is applied before the table exists.
    let dispatcher = f.deps.dispatcher.clone();
    f.vcx.update(|_, cx| {
        dispatcher.dispatch(
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: pods(),
            },
            cx,
        );
        dispatcher.dispatch(
            Command::TableSetFilter {
                cluster: cluster(),
                gvk: pods(),
                text: "^web".to_owned(),
            },
            cx,
        );
    });
    f.settle();
    let table = only_table(&mut f);
    assert_eq!(f.names(&table), ["web-1", "web-2"]);
    assert_eq!(bar_text(&mut f, &table), "^web");
}

#[gpui::test]
fn a_waiting_filter_expires(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    set_filter(&mut f, "api");
    // Nothing opened the table; the jump is long over.
    f.vcx
        .executor()
        .advance_clock(crate::views::PENDING_FILTER_TTL + std::time::Duration::from_secs(1));
    let table = f.open_pods();
    assert_eq!(f.names(&table).len(), 5);
    assert_eq!(bar_text(&mut f, &table), "");
}
