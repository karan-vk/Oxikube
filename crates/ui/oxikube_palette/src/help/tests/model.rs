//! The model without a window: grouping, order, search rows, chips.

use oxikube_domain::command::CommandCategory;
use oxikube_keymap::{ActiveBinding, BindingInfo, KeymapLayer, SuppressedBinding};

use super::super::{HelpCategory, HelpModel, HelpScope, HelpSource, HelpState, Row};

fn binding(action: &'static str, keys: &str, layer: Option<KeymapLayer>) -> ActiveBinding {
    ActiveBinding {
        action,
        binding: BindingInfo {
            keystrokes: keys.split_whitespace().map(str::to_owned).collect(),
            context: Some("ResourceTable && !Editing".to_owned()),
            layer,
        },
    }
}

fn focused() -> HelpScope {
    HelpScope::Focused {
        innermost: "ResourceTable".into(),
    }
}

fn model(active: Vec<ActiveBinding>) -> HelpModel {
    HelpModel::build(focused(), active, Vec::new())
}

fn all_rows(model: &HelpModel) -> Vec<Row> {
    model.search("")
}

fn headers(rows: &[Row]) -> Vec<&'static str> {
    rows.iter()
        .filter_map(|row| match row {
            Row::Header { category, .. } => Some(category.label()),
            Row::Entry { .. } => None,
        })
        .collect()
}

#[test]
fn bindings_of_one_category_sit_under_one_header_in_category_order() {
    let model = model(vec![
        binding("resource_table::ViewLogs", "l", None),
        binding("resource_table::ViewYaml", "y", None),
        binding("resource_table::SelectNext", "down", None),
        binding("resource_table::DeleteSelected", "ctrl-d", None),
        binding("resource_table::ViewDescribe", "d", None),
        binding("resource_table::ShellSelected", "s", None),
    ]);
    let rows = all_rows(&model);
    // Resource (yaml, describe, delete), Pod (logs, shell), then Navigation: S01's order.
    assert_eq!(headers(&rows), ["Resource", "Pod", "Navigation"]);
    let layout: Vec<String> = rows
        .iter()
        .map(|row| match row {
            Row::Header { category, count } => format!("# {} ({count})", category.label()),
            Row::Entry { entry, .. } => model.entries()[*entry].title.to_string(),
        })
        .collect();
    assert_eq!(layout[0], "# Resource (3)");
    // Titles come from the commands the view actions stand for, sorted inside a group.
    assert_eq!(
        &layout[1..4],
        ["Delete Resource", "Describe", "View YAML"]
            .map(|t| t.to_owned())
            .as_slice(),
        "{layout:?}"
    );
    assert_eq!(layout[4], "# Pod (2)");
    assert_eq!(layout.last().map(String::as_str), Some("Select next"));
}

#[test]
fn a_header_is_listed_only_when_it_has_entries_and_the_count_is_the_matches() {
    let model = model(vec![
        binding("resource_table::ViewYaml", "y", None),
        binding("resource_table::ViewLogs", "l", None),
        binding("resource_table::ViewDescribe", "d", None),
    ]);
    let rows = model.search("yaml");
    assert_eq!(headers(&rows), ["Resource"], "the Pod group has no match");
    assert!(matches!(rows[0], Row::Header { count: 1, .. }));
}

#[test]
fn a_query_orders_a_group_by_match_quality_and_highlights_the_title() {
    let model = model(vec![
        binding("resource_table::ViewDescribe", "d", None),
        binding("resource_table::ViewYaml", "y", None),
    ]);
    let rows = model.search("view y");
    let Row::Entry { entry, positions } = &rows[1] else {
        panic!("{rows:?}");
    };
    assert_eq!(
        model.entries()[*entry].title,
        "View YAML",
        "best match first"
    );
    assert!(!positions.is_empty());
    assert!(
        positions
            .iter()
            .all(|p| *p < model.entries()[*entry].title.len()),
        "only the title is highlighted"
    );
}

#[test]
fn search_finds_by_title_category_keystroke_and_action_name() {
    let model = model(vec![
        binding("resource_table::DeleteSelected", "ctrl-d", None),
        binding("resource_table::ViewYaml", "y", None),
        binding("resource_table::ViewLogs", "l", None),
    ]);
    let find = |query: &str| -> Vec<String> {
        model
            .search(query)
            .into_iter()
            .filter_map(|row| match row {
                Row::Entry { entry, .. } => Some(model.entries()[entry].title.to_string()),
                Row::Header { .. } => None,
            })
            .collect()
    };
    assert_eq!(find("delete"), ["Delete Resource"], "title");
    assert_eq!(find("ctrl-d"), ["Delete Resource"], "keystroke");
    assert_eq!(find("viewyaml"), ["View YAML"], "action name");
    assert_eq!(find("pod").len(), 1, "category: the Pod group's logs");
    assert!(find("zzzz").is_empty());
}

#[test]
fn the_source_chips_follow_the_layer() {
    let model = model(vec![
        binding("resource_table::ViewYaml", "x", Some(KeymapLayer::User)),
        binding("resource_table::ViewDescribe", "j", Some(KeymapLayer::Vim)),
        binding("resource_table::ViewLogs", "l", Some(KeymapLayer::Default)),
        binding("resource_table::ShellSelected", "s", None),
    ]);
    let source = |title: &str| {
        model
            .entries()
            .iter()
            .find(|e| e.title == title)
            .map(|e| e.source)
    };
    assert_eq!(source("View YAML"), Some(HelpSource::User));
    assert_eq!(source("Describe"), Some(HelpSource::Base));
    assert_eq!(source("View Logs"), Some(HelpSource::Default));
    assert_eq!(source("Shell"), Some(HelpSource::Default));
    // "user" finds the overrides first.
    let rows = model.search("user");
    let Some(Row::Entry { entry, .. }) = rows.iter().find(|row| matches!(row, Row::Entry { .. }))
    else {
        panic!("{rows:?}");
    };
    assert_eq!(model.entries()[*entry].title, "View YAML");
}

#[test]
fn a_binding_the_user_unbound_is_listed_as_such() {
    let hidden = SuppressedBinding {
        action: "resource_table::ViewDescribe",
        binding: BindingInfo {
            keystrokes: vec!["d".to_owned()],
            context: None,
            layer: Some(KeymapLayer::Default),
        },
        by: KeymapLayer::User,
    };
    let model = HelpModel::build(focused(), Vec::new(), vec![hidden]);
    let entry = &model.entries()[0];
    assert_eq!(entry.state, HelpState::Unbound(HelpSource::User));
    assert!(entry.haystack().contains("unbound"));
}

#[test]
fn a_context_with_no_bindings_has_no_rows() {
    let model = model(Vec::new());
    assert!(model.entries().is_empty());
    assert!(all_rows(&model).is_empty());
}

#[test]
fn a_long_list_groups_without_losing_an_entry() {
    let mut active = Vec::new();
    let names = [
        "resource_table::ViewYaml",
        "resource_table::ViewLogs",
        "resource_table::SelectNext",
        "log_view::ToggleWrap",
    ];
    for i in 0..2_000 {
        active.push(binding(
            names[i % names.len()],
            &format!("ctrl-alt-{i}"),
            None,
        ));
    }
    let model = model(active);
    let rows = all_rows(&model);
    let entries = rows
        .iter()
        .filter(|row| matches!(row, Row::Entry { .. }))
        .count();
    assert_eq!(entries, 2_000);
    assert!(headers(&rows).len() <= 6);
    // A one-word query is matched field by field (five per entry), several words on all fields.
    assert_eq!(model.candidates_for("yaml").len(), 2_000 * 5);
    assert_eq!(model.candidates_for("view yaml").len(), 2_000);
    assert_eq!(model.candidates_for("").len(), 2_000);
    assert!(
        model
            .entries()
            .windows(2)
            .all(|pair| pair[0].category <= pair[1].category)
    );
    let _ = HelpCategory::Command(CommandCategory::App);
}

#[test]
fn a_word_must_match_inside_one_field() {
    // "log": an `l` in Select, an `o` in Previous and a `g` in Navigation is no match; the log
    // viewer's action is.
    let model = model(vec![
        binding("resource_table::SelectPrevious", "up", None),
        binding("resource_table::ViewLogs", "l", None),
    ]);
    let rows = model.search("log");
    let titles: Vec<_> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Entry { entry, .. } => Some(model.entries()[*entry].title.to_string()),
            Row::Header { .. } => None,
        })
        .collect();
    assert_eq!(titles, ["View Logs"]);
    // Words of a longer query may match different fields: the title and the category.
    assert_eq!(
        model.search("view pod").len(),
        2,
        "one header and one entry"
    );
}
