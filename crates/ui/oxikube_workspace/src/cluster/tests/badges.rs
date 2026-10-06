//! The badge appears on the tab, the hotbar entry and the status bar with the flag, and goes
//! away with it; the colour and the lock are separate; one change redraws once.

use gpui::TestAppContext;
use oxikube_domain::{ClusterColour, ClusterPreset};

use super::{Fixture, bounds, fixture};
use crate::cluster::ClusterMark;
use crate::cluster::tests::fixture::{LAB, PROD, id};

fn set_read_only(f: &mut Fixture, on: bool) {
    f.manager.set_read_only(&id(PROD), on).unwrap();
    f.vcx.run_until_parked();
}

fn set_colour(f: &mut Fixture, colour: Option<ClusterColour>) {
    f.manager.set_colour(&id(PROD), colour).unwrap();
    f.vcx.run_until_parked();
}

#[gpui::test]
fn the_lock_shows_on_tab_hotbar_and_status_bar_with_the_flag(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    for lock in [
        "tab-prod-eu-badge-lock",
        "hotbar-lock",
        "status-cluster-badge-lock",
    ] {
        assert!(bounds(&mut f.vcx, lock).is_none(), "{lock} before the flag");
    }

    set_read_only(&mut f, true);
    for lock in [
        "tab-prod-eu-badge-lock",
        "hotbar-lock",
        "status-cluster-badge-lock",
    ] {
        assert!(bounds(&mut f.vcx, lock).is_some(), "{lock} with the flag");
    }
    assert!(bounds(&mut f.vcx, "status-cluster-read-only").is_some());

    set_read_only(&mut f, false);
    for lock in [
        "tab-prod-eu-badge-lock",
        "hotbar-lock",
        "status-cluster-badge-lock",
        "status-cluster-read-only",
    ] {
        assert!(bounds(&mut f.vcx, lock).is_none(), "{lock} after the flag");
    }
}

#[gpui::test]
fn the_colour_dot_is_separate_from_the_lock(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    set_colour(&mut f, Some(ClusterPreset::PROD_COLOUR));
    for dot in [
        "tab-prod-eu-badge-dot",
        "hotbar-dot",
        "status-cluster-badge-dot",
    ] {
        assert!(bounds(&mut f.vcx, dot).is_some(), "{dot}");
    }
    assert!(
        bounds(&mut f.vcx, "hotbar-lock").is_none(),
        "a colour alone is not read-only"
    );

    set_read_only(&mut f, true);
    assert!(bounds(&mut f.vcx, "hotbar-dot").is_some());
    assert!(bounds(&mut f.vcx, "hotbar-lock").is_some());

    set_colour(&mut f, None);
    assert!(bounds(&mut f.vcx, "hotbar-dot").is_none());
    assert!(
        bounds(&mut f.vcx, "hotbar-lock").is_some(),
        "read-only without a colour still shows the lock"
    );
}

#[gpui::test]
fn the_marks_follow_the_session(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    set_read_only(&mut f, true);
    set_colour(&mut f, Some(ClusterPreset::STAGING_COLOUR));
    let want = ClusterMark {
        colour: Some(ClusterPreset::STAGING_COLOUR),
        read_only: true,
    };
    assert_eq!(f.vcx.update(|_, cx| f.status.read(cx).mark()), want);
    let tab = f.tab.clone();
    let content = f.vcx.update(|_, cx| {
        use crate::item::Item as _;
        tab.read(cx).tab_content(cx)
    });
    assert_eq!(content.cluster, Some(want));
}

#[gpui::test]
fn another_clusters_changes_do_not_wake_the_views(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let seen = notified.clone();
    let _sub = f
        .vcx
        .update(|_, cx| cx.observe(&f.status, move |_, _| seen.set(seen.get() + 1)));
    f.manager.set_read_only(&id(LAB), true).unwrap();
    f.manager
        .set_colour(&id(LAB), Some(ClusterPreset::DEV_COLOUR))
        .unwrap();
    f.vcx.run_until_parked();
    assert_eq!(notified.get(), 0, "no redraw for another cluster");
    assert!(bounds(&mut f.vcx, "status-cluster-badge-lock").is_none());
}

#[gpui::test]
fn a_preset_that_changes_two_fields_redraws_the_status_item_once(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let seen = notified.clone();
    let _sub = f
        .vcx
        .update(|_, cx| cx.observe(&f.status, move |_, _| seen.set(seen.get() + 1)));
    // Colour and read-only change back to back, as a preset does.
    f.manager
        .set_colour(&id(PROD), Some(ClusterPreset::PROD_COLOUR))
        .unwrap();
    f.manager.set_read_only(&id(PROD), true).unwrap();
    f.vcx.run_until_parked();
    assert_eq!(notified.get(), 1, "one redraw for the whole burst");
}

#[gpui::test]
fn the_status_item_is_hidden_without_an_active_cluster_and_follows_the_name(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    assert!(bounds(&mut f.vcx, "status-cluster").is_some());
    f.status
        .update(&mut f.vcx, |item, cx| item.set_cluster(None, cx));
    f.vcx.run_until_parked();
    assert!(bounds(&mut f.vcx, "status-cluster").is_none());

    f.status
        .update(&mut f.vcx, |item, cx| item.set_cluster(Some(id(LAB)), cx));
    f.vcx.run_until_parked();
    assert!(bounds(&mut f.vcx, "status-cluster").is_some());
}
