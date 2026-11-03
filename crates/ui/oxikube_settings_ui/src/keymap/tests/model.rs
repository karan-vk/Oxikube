//! The list model: search, filters, empty states and the cost of filtering 2 000 rows.

use std::time::Instant;

use oxikube_keymap::{KeymapLayer, KeymapRow, RowStatus};

use crate::keymap::{
    ContextFilter, EmptyState, KeymapListModel, Query, RowKey, canonical_keystroke,
};

pub(super) fn row(
    title: &str,
    action: &str,
    context: Option<&str>,
    keys: &[&str],
    source: Option<KeymapLayer>,
    status: RowStatus,
) -> KeymapRow {
    KeymapRow {
        action: action.to_owned(),
        title: title.to_owned(),
        context: context.map(str::to_owned),
        keystrokes: keys.iter().map(|k| (*k).to_owned()).collect(),
        data: None,
        source,
        status,
    }
}

fn sample() -> Vec<KeymapRow> {
    use KeymapLayer::{Default, User};
    vec![
        row(
            "Open Palette",
            "palette::Toggle",
            None,
            &["ctrl-k"],
            Some(Default),
            RowStatus::Active,
        ),
        row(
            "Scroll Down",
            "table::SelectNext",
            Some("ResourceTable"),
            &["ctrl-d"],
            Some(Default),
            RowStatus::Active,
        ),
        row(
            "Scroll Down",
            "table::SelectNext",
            Some("ResourceTable"),
            &["ctrl-shift-d"],
            Some(User),
            RowStatus::Active,
        ),
        row(
            "Delete Pod",
            "pod::Delete",
            Some("ResourceTable"),
            &["ctrl-x"],
            Some(Default),
            RowStatus::Overridden,
        ),
        row(
            "Show Logs",
            "logs::Show",
            Some("Workspace"),
            &["l"],
            Some(Default),
            RowStatus::UnboundByUser,
        ),
        row(
            "Restart",
            "workload::Restart",
            None,
            &[],
            None,
            RowStatus::Unbound,
        ),
    ]
}

fn titles(model: &KeymapListModel) -> Vec<String> {
    (0..model.len())
        .map(|ix| {
            let row = model.row(ix).unwrap();
            format!("{}:{}", row.title, row.keystrokes_text())
        })
        .collect()
}

#[test]
fn everything_shows_by_default_unbound_commands_included() {
    let model = KeymapListModel::new(sample());
    assert_eq!(model.len(), 6);
    assert_eq!(model.empty_state(), EmptyState::None);
    assert!(model.has_user_bindings());
}

#[test]
fn the_search_matches_title_action_id_context_and_keystrokes() {
    let mut model = KeymapListModel::new(sample());
    for (query, expected) in [
        ("palette", 1),
        ("pod::delete", 1),
        ("resourcetable", 3),
        ("scroll down", 2),
        ("scroll workspace", 0),
        // The binding without a context, not the command bound nowhere.
        ("everywhere", 1),
        ("ctrl-k", 1),
    ] {
        model.set_query(query);
        assert_eq!(model.len(), expected, "{query}");
    }
}

#[test]
fn a_keystroke_matches_however_it_is_spelled() {
    let mut model = KeymapListModel::new(sample());
    for spelling in ["Ctrl+D", "ctrl-d", "CTRL-D"] {
        model.set_query(spelling);
        assert_eq!(titles(&model), ["Scroll Down:ctrl-d"], "{spelling}");
    }
    // Modifier order does not matter: `shift-ctrl-d` is the keymap's `ctrl-shift-d`.
    model.set_query("shift-ctrl-d");
    assert_eq!(titles(&model), ["Scroll Down:ctrl-shift-d"]);
    model.set_query("Ctrl+Shift+D");
    assert_eq!(titles(&model), ["Scroll Down:ctrl-shift-d"]);
}

#[test]
fn canonical_keystrokes_are_the_keymaps_spelling() {
    assert_eq!(canonical_keystroke("ctrl+d").as_deref(), Some("ctrl-d"));
    assert_eq!(
        canonical_keystroke("shift-ctrl-d").as_deref(),
        Some("ctrl-shift-d")
    );
    assert_eq!(
        canonical_keystroke("ctrl"),
        None,
        "a modifier alone is not a keystroke"
    );
    assert_eq!(canonical_keystroke("").as_deref(), None);
    assert!(Query::parse("   ").is_empty());
}

#[test]
fn the_context_filter_limits_rows_and_hides_unbound_commands() {
    let mut model = KeymapListModel::new(sample());
    assert_eq!(model.contexts(), ["ResourceTable", "Workspace"]);
    assert!(model.has_everywhere());

    model.set_context(ContextFilter::Named("ResourceTable".into()));
    assert_eq!(model.len(), 3);
    model.set_context(ContextFilter::Everywhere);
    assert_eq!(
        titles(&model),
        ["Open Palette:ctrl-k"],
        "unbound commands have no context"
    );
    model.set_context(ContextFilter::All);
    assert_eq!(model.len(), 6);
}

#[test]
fn show_unbound_hides_the_commands_nothing_is_bound_to() {
    let mut model = KeymapListModel::new(sample());
    model.set_show_unbound(false);
    assert_eq!(model.len(), 5);
    assert!(titles(&model).iter().all(|t| !t.starts_with("Restart")));
    model.set_show_unbound(true);
    assert_eq!(model.len(), 6);
}

#[test]
fn empty_states_tell_no_keymap_from_no_matches() {
    let mut model = KeymapListModel::new(Vec::new());
    assert_eq!(model.empty_state(), EmptyState::NoKeymap);
    model.set_rows(sample());
    model.set_query("zzz");
    assert_eq!(model.empty_state(), EmptyState::NoMatches);
    model.set_query("");
    assert_eq!(model.empty_state(), EmptyState::None);
}

#[test]
fn a_reload_keeps_search_and_filters_and_resets_a_filter_whose_context_is_gone() {
    let mut model = KeymapListModel::new(sample());
    model.set_query("scroll");
    model.set_context(ContextFilter::Named("ResourceTable".into()));
    let mut rows = sample();
    rows.push(row(
        "Scroll Up",
        "table::SelectPrev",
        Some("ResourceTable"),
        &["ctrl-u"],
        Some(KeymapLayer::Default),
        RowStatus::Active,
    ));
    model.set_rows(rows);
    assert_eq!(model.len(), 3);
    assert_eq!(model.query_text(), "scroll");

    model.set_rows(vec![sample().remove(0)]);
    assert_eq!(
        model.context(),
        &ContextFilter::All,
        "no ResourceTable rows any more"
    );
}

#[test]
fn a_row_is_found_again_by_its_key_after_a_reload() {
    let mut model = KeymapListModel::new(sample());
    let key = RowKey::of(model.row(2).unwrap());
    let mut rows = sample();
    rows.insert(
        0,
        row(
            "Aardvark",
            "a::B",
            None,
            &["f1"],
            Some(KeymapLayer::User),
            RowStatus::Active,
        ),
    );
    model.set_rows(rows);
    assert_eq!(model.position_of(&key), Some(3));
}

#[test]
fn filtering_two_thousand_rows_is_far_inside_the_five_millisecond_budget() {
    let rows: Vec<KeymapRow> = (0..2000)
        .map(|n| {
            row(
                &format!("Command number {n}"),
                &format!("ns{}::Action{n}", n % 40),
                Some(&format!("Context{}", n % 25)),
                &["ctrl-shift-k", "ctrl-v"][..1 + n % 2],
                Some(KeymapLayer::Default),
                RowStatus::Active,
            )
        })
        .collect();
    let mut model = KeymapListModel::new(rows);
    // Median of many runs, over queries that match many, few and no rows.
    let mut samples = Vec::new();
    for query in [
        "command",
        "ctrl-v",
        "Ctrl+Shift+K",
        "zzz",
        "ns7 context7",
        "",
    ] {
        for _ in 0..20 {
            model.set_query("x");
            let start = Instant::now();
            model.set_query(query);
            samples.push(start.elapsed());
        }
    }
    samples.sort();
    let p95 = samples[samples.len() * 95 / 100];
    eprintln!("keymap filter, 2000 rows: p95 {p95:?} (debug build)");
    // The budget is 5 ms in release; an unoptimised build gets ten times that.
    let budget = if cfg!(debug_assertions) { 50 } else { 5 };
    assert!(p95.as_millis() < budget, "p95 {p95:?}");
}

#[test]
fn a_row_key_tells_apart_rows_that_differ_only_in_data_or_status() {
    let mut first = row(
        "Activate Tab",
        "tab::Activate",
        Some("Workspace"),
        &["ctrl-1"],
        Some(KeymapLayer::User),
        RowStatus::Overridden,
    );
    first.data = Some("1".to_owned());
    let mut second = first.clone();
    second.data = Some("2".to_owned());
    second.status = RowStatus::Active;
    let model = KeymapListModel::new(vec![first, second]);
    let key = RowKey::of(model.row(1).expect("two rows"));
    assert_eq!(model.position_of(&key), Some(1));
    assert!(key.is(model.row(1).expect("two rows")));
    assert!(!key.is(model.row(0).expect("two rows")));
}
