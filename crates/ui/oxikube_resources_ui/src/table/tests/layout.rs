//! [`ColumnLayout`]: defaults, saved order / visibility / widths / sort, and what is saved back.

use std::sync::Arc;

use oxikube_app::{Column, ColumnId, ColumnProvider, CoreColumns};
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;

use crate::table::{ColumnLayout, ColumnPrefs, SavedSort};

fn pod_columns() -> Arc<[Column]> {
    CoreColumns::new().columns(&Gvk::new("", "v1", "Pod"), Capabilities::empty())
}

fn ids(layout: &ColumnLayout) -> Vec<String> {
    layout
        .visible_ids()
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn defaults_show_the_default_columns_in_provider_order() {
    let all = pod_columns();
    let layout = ColumnLayout::new(all.clone(), &ColumnPrefs::default());
    let expected: Vec<String> = all
        .iter()
        .filter(|c| !c.wide)
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(ids(&layout), expected);
    assert!(
        !layout.is_shown(&ColumnId::new("qos")),
        "wide columns start hidden"
    );
    assert_eq!(layout.sort(), None);
}

#[test]
fn saved_order_visibility_widths_and_sort_apply_and_round_trip() {
    let prefs = ColumnPrefs {
        order: vec!["status".into(), "name".into(), "gone".into()],
        visible: [("qos".to_owned(), true), ("node".to_owned(), false)].into(),
        widths: [("name".to_owned(), 321.0), ("gone".to_owned(), 99.0)].into(),
        sort: Some(SavedSort {
            column: "restarts".into(),
            descending: true,
        }),
        ..ColumnPrefs::default()
    };
    let layout = ColumnLayout::new(pod_columns(), &prefs);
    let shown = ids(&layout);
    assert_eq!(&shown[..2], ["status", "name"], "the saved order first");
    assert!(
        shown.contains(&"qos".to_owned()),
        "a wide column the user showed"
    );
    assert!(
        !shown.contains(&"node".to_owned()),
        "a default column the user hid"
    );
    assert_eq!(layout.width(&ColumnId::new("name")), Some(321.0));
    assert_eq!(layout.sort(), Some(&(ColumnId::new("restarts"), true)));

    let saved = layout.prefs();
    assert_eq!(
        saved.widths.get("gone"),
        Some(&99.0),
        "choices for absent columns survive"
    );
    assert_eq!(ColumnLayout::new(pod_columns(), &saved), layout);
}

#[test]
fn the_name_column_and_the_last_column_cannot_be_hidden() {
    let mut layout = ColumnLayout::new(pod_columns(), &ColumnPrefs::default());
    assert!(!layout.set_shown(&ColumnId::new("name"), false));
    for id in layout.visible_ids() {
        layout.set_shown(&id, false);
    }
    assert_eq!(ids(&layout), ["name"]);
}

#[test]
fn hiding_the_sort_column_drops_the_sort() {
    let mut layout = ColumnLayout::new(pod_columns(), &ColumnPrefs::default());
    layout.set_sort(Some((ColumnId::new("restarts"), false)));
    assert!(layout.set_shown(&ColumnId::new("restarts"), false));
    assert_eq!(layout.sort(), None);
}

#[test]
fn moving_a_visible_column_matches_a_header_drag() {
    let mut layout = ColumnLayout::new(pod_columns(), &ColumnPrefs::default());
    let before = ids(&layout);
    assert!(layout.move_visible(0, 2));
    let mut expected = before;
    let moved = expected.remove(0);
    expected.insert(2, moved);
    assert_eq!(ids(&layout), expected);
    assert!(layout.move_visible(3, 1));
    let moved = expected.remove(3);
    expected.insert(1, moved);
    assert_eq!(ids(&layout), expected);
    assert!(!layout.move_visible(0, 99));
}

#[test]
fn widths_record_only_real_changes() {
    let mut layout = ColumnLayout::new(pod_columns(), &ColumnPrefs::default());
    let defaults: Vec<f32> = (0..layout.visible_len())
        .map(|ix| ColumnLayout::default_width(layout.visible(ix).unwrap()))
        .collect();
    assert!(!layout.set_widths(&defaults));
    let mut wider = defaults.clone();
    wider[1] += 40.;
    assert!(layout.set_widths(&wider));
    let id = layout.visible(1).unwrap().id.clone();
    assert_eq!(layout.width(&id), Some(wider[1]));
    assert_eq!(layout.prefs().widths.len(), 1);
}

/// The generic Name / Namespace / Age columns a Table-feed kind starts with.
fn generic_columns() -> Arc<[Column]> {
    oxikube_app::TableColumns::new(
        &[],
        oxikube_ports::TableSource::Objects,
        oxikube_domain::ids::Scope::Namespaced,
    )
    .columns(&Gvk::new("", "v1", "Pod"), Capabilities::empty())
}

#[test]
fn order_and_sort_on_absent_columns_survive_a_narrower_column_set() {
    // Saved over the full pod columns: status first, sorted by restarts.
    let mut full = ColumnLayout::new(pod_columns(), &ColumnPrefs::default());
    let status = full.visible_index(&ColumnId::new("status")).unwrap();
    assert!(full.move_visible(status, 0));
    full.set_sort(Some((ColumnId::new("restarts"), true)));
    let saved = full.prefs();

    // Read back over the generic columns first (the feed's columns have not arrived), then
    // rebuilt from that layout's prefs when they do.
    let narrow = ColumnLayout::new(generic_columns(), &saved);
    assert_eq!(narrow.sort(), None, "no restarts column to sort by yet");
    let resaved = narrow.prefs();
    assert_eq!(
        resaved.order, saved.order,
        "absent columns keep their slots"
    );
    assert_eq!(
        resaved.sort, saved.sort,
        "the sort on an absent column is kept"
    );
    let restored = ColumnLayout::new(pod_columns(), &resaved);
    assert_eq!(restored, full);
    assert_eq!(ids(&restored)[0], "status");
    assert_eq!(restored.sort(), Some(&(ColumnId::new("restarts"), true)));
}

#[test]
fn reordering_a_narrower_layout_keeps_the_absent_slots() {
    let saved = ColumnPrefs {
        order: vec![
            "status".into(),
            "name".into(),
            "age".into(),
            "namespace".into(),
        ],
        ..ColumnPrefs::default()
    };
    let mut narrow = ColumnLayout::new(generic_columns(), &saved);
    assert_eq!(ids(&narrow), ["name", "age", "namespace"]);
    assert!(narrow.move_visible(2, 0));
    assert_eq!(
        narrow.prefs().order,
        ["status", "namespace", "name", "age"],
        "the present columns take the slots in their new order"
    );
}

#[test]
fn choosing_a_sort_forgets_a_saved_sort_on_an_absent_column() {
    let saved = ColumnPrefs {
        sort: Some(SavedSort {
            column: "restarts".into(),
            descending: true,
        }),
        ..ColumnPrefs::default()
    };
    let mut narrow = ColumnLayout::new(generic_columns(), &saved);
    assert!(narrow.set_sort(None), "the user chose the default order");
    assert_eq!(narrow.prefs().sort, None);
    let mut narrow = ColumnLayout::new(generic_columns(), &saved);
    narrow.set_sort(Some((ColumnId::new("age"), false)));
    assert_eq!(
        narrow.prefs().sort.map(|s| s.column),
        Some("age".to_owned())
    );
}

#[test]
fn toggle_wide_shows_every_wide_column_then_hides_them_again() {
    let all = pod_columns();
    let wide: Vec<ColumnId> = all
        .iter()
        .filter(|c| c.wide)
        .map(|c| c.id.clone())
        .collect();
    assert!(wide.len() > 1, "pods have wide columns");
    let mut layout = ColumnLayout::new(all, &ColumnPrefs::default());
    let default_shown = ids(&layout);
    assert!(!layout.wide_shown());

    assert!(layout.toggle_wide(), "the first toggle changes the layout");
    assert!(layout.wide_shown());
    for id in &wide {
        assert!(layout.is_shown(id), "{id} shows");
    }

    assert!(layout.toggle_wide());
    assert!(!layout.wide_shown());
    assert_eq!(ids(&layout), default_shown, "back to the default columns");
}

#[test]
fn toggle_wide_with_one_wide_column_shown_shows_the_rest() {
    // The user showed `qos` from the picker; `ctrl-w` completes the set, it does not hide it.
    let prefs = ColumnPrefs {
        visible: [("qos".to_owned(), true)].into(),
        ..ColumnPrefs::default()
    };
    let mut layout = ColumnLayout::new(pod_columns(), &prefs);
    assert!(layout.is_shown(&ColumnId::new("qos")) && !layout.wide_shown());
    assert!(layout.toggle_wide());
    assert!(layout.wide_shown());
}

#[test]
fn toggle_wide_keeps_the_default_columns_and_the_users_order() {
    let prefs = ColumnPrefs {
        order: vec!["status".into(), "name".into()],
        visible: [("node".to_owned(), false)].into(),
        ..ColumnPrefs::default()
    };
    let mut layout = ColumnLayout::new(pod_columns(), &prefs);
    let before = ids(&layout);
    layout.toggle_wide();
    let after = ids(&layout);
    assert_eq!(&after[..2], ["status", "name"]);
    assert!(
        !after.contains(&"node".to_owned()),
        "a default column the user hid stays hidden"
    );
    assert!(after.len() > before.len());
    layout.toggle_wide();
    assert_eq!(ids(&layout), before);
}

#[test]
fn toggle_wide_drops_a_sort_on_a_column_it_hides() {
    let prefs = ColumnPrefs {
        visible: [("qos".to_owned(), true)].into(),
        sort: Some(SavedSort {
            column: "qos".into(),
            descending: false,
        }),
        ..ColumnPrefs::default()
    };
    let mut layout = ColumnLayout::new(pod_columns(), &prefs);
    assert!(layout.sort().is_some());
    layout.toggle_wide(); // shows the rest
    assert!(layout.sort().is_some());
    layout.toggle_wide(); // hides them all, the sorted one too
    assert_eq!(layout.sort(), None);
}

#[test]
fn toggle_wide_does_nothing_for_a_kind_without_wide_columns() {
    let narrow: Arc<[Column]> = pod_columns().iter().filter(|c| !c.wide).cloned().collect();
    let mut layout = ColumnLayout::new(narrow, &ColumnPrefs::default());
    let before = layout.clone();
    assert!(!layout.toggle_wide());
    assert!(!layout.wide_shown());
    assert_eq!(layout, before);
}
