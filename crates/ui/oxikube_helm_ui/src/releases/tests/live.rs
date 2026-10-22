//! The list follows the cluster: a new revision appears without a poll, the open detail follows
//! its release, and a release that is gone closes it.

use gpui::TestAppContext;
use oxikube_domain::helm::ReleaseStatus;

use super::{Data, Harness, full, release};
use crate::releases::SETTLE;

fn data() -> Data {
    Data {
        releases: vec![
            release("apps", "web", ReleaseStatus::Deployed, 1),
            release("apps", "db", ReleaseStatus::Deployed, 5),
        ],
        ..Data::default()
    }
}

fn settle_burst(h: &mut Harness) {
    h.vcx.executor().advance_clock(SETTLE * 2);
    h.settle();
}

#[gpui::test]
fn a_new_revision_shows_up_after_a_change_and_not_before(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(), full());
    assert_eq!(h.calls(), ["list"]);
    // Nothing changed: no timer re-reads.
    settle_burst(&mut h);
    h.vcx
        .executor()
        .advance_clock(std::time::Duration::from_secs(120));
    h.settle();
    assert_eq!(h.calls(), ["list"], "no polling");

    // `helm upgrade web` outside the app: revision 2.
    h.backend.data.lock().releases[0] = release("apps", "web", ReleaseStatus::Deployed, 2);
    h.changed.unbounded_send(()).unwrap();
    settle_burst(&mut h);
    assert_eq!(h.calls(), ["list", "list"]);
    let revision = h.read(|v, cx| {
        v.table.read(cx, |d| {
            d.find(&super::release_ref("web")).map(|r| r.revision)
        })
    });
    assert_eq!(revision, Some(2));
}

#[gpui::test]
fn a_burst_of_changes_is_one_list(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(), full());
    for _ in 0..6 {
        h.changed.unbounded_send(()).unwrap();
    }
    settle_burst(&mut h);
    assert_eq!(h.calls(), ["list", "list"]);
}

#[gpui::test]
fn the_open_detail_follows_its_release_to_the_new_revision(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(), full());
    h.select(0);
    let before = h
        .calls()
        .iter()
        .filter(|c| c.starts_with("objects:"))
        .count();
    assert_eq!(before, 1);
    h.backend.data.lock().releases[0] = release("apps", "web", ReleaseStatus::Deployed, 2);
    h.changed.unbounded_send(()).unwrap();
    settle_burst(&mut h);
    assert_eq!(h.detail(|d| d.release.revision), 2);
    assert!(
        h.calls().contains(&"objects:web:2".to_owned()),
        "the shown tab was read again for the new revision: {:?}",
        h.calls()
    );
}

#[gpui::test]
fn a_release_that_is_uninstalled_closes_its_detail(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(), full());
    h.select(0);
    assert!(h.read(|v, _| v.detail.is_some()));
    h.backend.data.lock().releases.remove(0);
    h.changed.unbounded_send(()).unwrap();
    settle_burst(&mut h);
    assert!(h.read(|v, _| v.detail.is_none()));
    assert_eq!(h.names(), ["apps/db"]);
}

#[gpui::test]
fn a_filtered_out_release_keeps_its_detail(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(), full());
    h.select(0);
    let view = h.view.clone();
    h.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.set_filter_text("db", cx)));
    h.changed.unbounded_send(()).unwrap();
    settle_burst(&mut h);
    assert!(
        h.read(|v, _| v.detail.is_some()),
        "filtered out is not gone"
    );
}

#[gpui::test]
fn a_change_during_a_list_lists_once_more(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(), full());
    let view = h.view.clone();
    h.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.refresh(cx);
            v.refresh(cx);
            v.refresh(cx);
        });
    });
    h.settle();
    // After the first list: one running and one queued for the other two requests, not three.
    assert_eq!(h.calls(), ["list", "list", "list"]);
}
