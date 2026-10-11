//! The `-s` status filter (E12-S07) in a table: what a click on a bar of the Workloads overview
//! types. It filters the cached rows by status bucket, composes with the namespace selection and
//! is cleared like any other filter.

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::pod;

use super::fixture::{Fixture, cluster};

fn fixtures() -> Vec<Resource> {
    vec![
        pod().namespace("x").name("ok-1").running().build(),
        pod().namespace("x").name("ok-2").running().build(),
        pod().namespace("x").name("waiting").pending().build(),
        pod().namespace("y").name("broken").failed().build(),
        pod().namespace("y").name("done").succeeded().build(),
    ]
}

fn set_filter(f: &mut Fixture, text: &str) {
    let dispatcher = f.deps.dispatcher.clone();
    let command = Command::TableSetFilter {
        cluster: cluster(),
        gvk: Gvk::new("", "v1", "Pod"),
        text: text.to_owned(),
    };
    f.vcx.update(|_, cx| dispatcher.dispatch(command, cx));
    f.settle();
}

#[gpui::test]
fn a_status_filter_keeps_the_pods_in_the_bucket(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    let table = f.open_pods();
    assert_eq!(f.names(&table).len(), 5);

    set_filter(&mut f, "-s pending");
    assert_eq!(f.names(&table), ["waiting"]);
    set_filter(&mut f, "-s running");
    assert_eq!(f.names(&table), ["ok-1", "ok-2"]);
    set_filter(&mut f, "-s failed");
    assert_eq!(f.names(&table), ["broken"]);
    set_filter(&mut f, "-s succeeded");
    assert_eq!(f.names(&table), ["done"]);

    set_filter(&mut f, "");
    assert_eq!(f.names(&table).len(), 5, "cleared like any filter");
}

#[gpui::test]
fn a_status_no_pod_is_in_shows_no_rows_and_a_typo_keeps_the_last_filter(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with(fixtures());
    let table = f.open_pods();
    set_filter(&mut f, "-s unknown");
    assert!(f.names(&table).is_empty(), "no pod has an unknown phase");
    set_filter(&mut f, "-s pending");
    set_filter(&mut f, "-s pendin");
    assert_eq!(
        f.names(&table),
        ["waiting"],
        "an unknown status is an error that keeps the last good filter"
    );
}
