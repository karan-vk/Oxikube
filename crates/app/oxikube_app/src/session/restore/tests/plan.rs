//! The pure part: matching a saved session against the catalog.

use super::*;
use crate::session::restore::RestorePlan;

#[test]
fn nothing_saved_plans_nothing() {
    let plan = RestorePlan::resolve(None, &[ctx("a")]);
    assert!(plan.is_empty() && plan.dropped.is_empty() && plan.active.is_none());
}

#[test]
fn saved_clusters_keep_their_order_and_the_displayed_one() {
    let saved = SavedTabs::new(vec![id("c"), id("a"), id("b")], Some(id("a")));
    let plan = RestorePlan::resolve(Some(&saved), &[ctx("a"), ctx("b"), ctx("c")]);

    assert_eq!(
        plan.ids().cloned().collect::<Vec<_>>(),
        [id("c"), id("a"), id("b")]
    );
    assert_eq!(plan.active, Some(id("a")));
    assert!(plan.dropped.is_empty());
}

#[test]
fn a_cluster_in_no_kubeconfig_is_dropped_with_its_name() {
    let saved = SavedTabs::new(vec![id("a"), id("gone"), id("b")], Some(id("gone")))
        .with_titles([(id("gone"), "Staging".to_owned())]);
    let plan = RestorePlan::resolve(Some(&saved), &[ctx("a"), ctx("b")]);

    assert_eq!(plan.ids().cloned().collect::<Vec<_>>(), [id("a"), id("b")]);
    assert_eq!(
        plan.active, None,
        "the displayed cluster is gone: the catalog home shows"
    );
    assert_eq!(plan.dropped.len(), 1);
    assert_eq!(plan.dropped[0].cluster, id("gone"));
    assert_eq!(plan.dropped[0].label(), "Staging");
}

#[test]
fn a_dropped_cluster_without_a_saved_name_is_labelled_by_id() {
    let saved = SavedTabs::new(vec![id("gone")], None);
    let plan = RestorePlan::resolve(Some(&saved), &[]);
    assert_eq!(plan.dropped[0].label(), id("gone").to_string());
}

#[test]
fn a_cluster_saved_twice_reopens_once() {
    let saved = SavedTabs::new(vec![id("a"), id("b"), id("a")], None);
    let plan = RestorePlan::resolve(Some(&saved), &[ctx("a"), ctx("b")]);
    assert_eq!(plan.ids().cloned().collect::<Vec<_>>(), [id("a"), id("b")]);
}

#[test]
fn pruning_keeps_the_survivors_and_their_names() {
    let saved = SavedTabs::new(vec![id("a"), id("gone")], Some(id("a")))
        .with_titles([(id("a"), "A".to_owned()), (id("gone"), "G".to_owned())]);
    let plan = RestorePlan::resolve(Some(&saved), &[ctx("a")]);
    let pruned = plan.pruned(&saved);

    assert_eq!(pruned.open, [id("a")]);
    assert_eq!(pruned.active, Some(id("a")));
    assert_eq!(pruned.title(&id("a")), Some("A"));
    assert_eq!(pruned.title(&id("gone")), None);
}

#[test]
fn rows_saved_before_names_existed_still_load() {
    let old = serde_json::json!({ "version": 1, "open": [id("a")], "active": null });
    let saved: SavedTabs = serde_json::from_value(old).expect("an old row");
    assert!(saved.titles.is_empty());
    assert_eq!(saved.open, [id("a")]);
}
