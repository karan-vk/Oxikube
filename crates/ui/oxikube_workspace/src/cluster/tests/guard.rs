//! The guard-mode status item: shows "Guarded" or "Read-only", follows the session on events
//! only, and a click runs `cluster::ToggleReadOnly` through the command runner.

use gpui::TestAppContext;
use oxikube_domain::ClusterPreset;

use super::fixture::{LAB, PROD, id};
use super::{bounds, fixture};
use crate::cluster::GuardMode;

fn mode(f: &mut super::Fixture) -> GuardMode {
    f.vcx.update(|_, cx| f.guard.read(cx).mode())
}

#[gpui::test]
fn the_item_follows_the_read_only_flag(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    assert_eq!(mode(&mut f), GuardMode::Guarded);
    assert!(bounds(&mut f.vcx, "status-guard").is_some());
    assert!(bounds(&mut f.vcx, "status-guard-lock").is_none());

    f.manager.set_read_only(&id(PROD), true).unwrap();
    f.vcx.run_until_parked();
    assert_eq!(mode(&mut f), GuardMode::ReadOnly);
    assert!(bounds(&mut f.vcx, "status-guard-lock").is_some());

    f.manager.set_read_only(&id(PROD), false).unwrap();
    f.vcx.run_until_parked();
    assert_eq!(mode(&mut f), GuardMode::Guarded);
    assert!(bounds(&mut f.vcx, "status-guard-lock").is_none());
}

#[gpui::test]
fn it_is_hidden_without_an_active_cluster_and_ignores_other_clusters(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    f.manager.set_read_only(&id(LAB), true).unwrap();
    f.vcx.run_until_parked();
    assert_eq!(mode(&mut f), GuardMode::Guarded, "another cluster's flag");

    f.guard
        .update(&mut f.vcx, |item, cx| item.set_cluster(None, cx));
    f.vcx.run_until_parked();
    assert!(bounds(&mut f.vcx, "status-guard").is_none());
}

#[gpui::test]
fn a_burst_of_changes_redraws_once(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let seen = notified.clone();
    let _sub = f
        .vcx
        .update(|_, cx| cx.observe(&f.guard, move |_, _| seen.set(seen.get() + 1)));
    f.manager
        .set_colour(&id(PROD), ClusterPreset::PROD_COLOUR.into())
        .unwrap();
    f.manager.set_read_only(&id(PROD), true).unwrap();
    f.vcx.run_until_parked();
    assert_eq!(notified.get(), 1);
}

#[gpui::test]
fn a_click_makes_the_cluster_read_only_and_a_second_click_lifts_it(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let item = bounds(&mut f.vcx, "status-guard").expect("drawn");
    f.vcx.simulate_click(item.center(), gpui::Modifiers::none());
    f.vcx.run_until_parked();
    assert!(f.manager.get(&id(PROD)).unwrap().read_only());
    assert_eq!(mode(&mut f), GuardMode::ReadOnly);

    // Lifting always asks first (the guard's rule, not the item's): nothing changes until the
    // dialog is answered.
    let item = bounds(&mut f.vcx, "status-guard").expect("drawn");
    f.vcx.simulate_click(item.center(), gpui::Modifiers::none());
    f.vcx.run_until_parked();
    let layer = f.vcx.update(|_, cx| f.ws.read(cx).modal_layer().clone());
    assert!(f.vcx.update(|_, cx| layer.read(cx).has_active_modal()));
    assert!(f.manager.get(&id(PROD)).unwrap().read_only(), "not yet");
    f.vcx.simulate_keystrokes("escape");
    f.vcx.run_until_parked();
    assert!(f.manager.get(&id(PROD)).unwrap().read_only(), "cancelled");
}
