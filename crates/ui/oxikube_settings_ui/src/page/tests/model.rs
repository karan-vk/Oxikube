//! The page model built from the schema: sections, fields, kinds, titles, descriptions.

use serde_json::json;

use crate::page::test_support::fixture_model;
use crate::page::{FieldKind, GENERAL_KEY, SettingsPageModel, humanize};

#[test]
fn general_comes_first_then_the_object_sections_by_title() {
    let model = fixture_model();
    let keys: Vec<&str> = model.sections().iter().map(|s| s.key.as_ref()).collect();
    assert_eq!(keys, [GENERAL_KEY, "editor", "terminal"]);
    let titles: Vec<&str> = model.sections().iter().map(|s| s.title.as_ref()).collect();
    assert_eq!(titles, ["General", "Manifest editor", "Terminal"]);
}

#[test]
fn clusters_and_the_schema_key_are_not_settings() {
    let model = fixture_model();
    assert!(model.find_field("clusters").is_none());
    assert!(model.find_field("$schema").is_none());
    assert!(model.section_index("clusters").is_none());
}

#[test]
fn top_level_scalars_are_general_fields_with_their_kinds() {
    let model = fixture_model();
    let general = model.section(0).expect("general");
    let kinds: Vec<(&str, &FieldKind)> = general
        .fields
        .iter()
        .map(|field| (field.path.as_ref(), &field.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            (
                "base_keymap",
                &FieldKind::Choice(vec!["default".into(), "vim".into()])
            ),
            ("confirm_quit", &FieldKind::Toggle),
            ("theme", &FieldKind::Other),
            ("ui_scale", &FieldKind::Number),
        ]
    );
}

#[test]
fn section_fields_have_paths_titles_descriptions_and_kinds() {
    let model = fixture_model();
    let (section, field) = model.find_field("terminal.font_size").expect("font size");
    let font_size = model.field(section, field).expect("the field");
    assert_eq!(font_size.title, "Font size");
    assert_eq!(font_size.description, "Font size of terminals, in points.");
    assert_eq!(font_size.kind, FieldKind::Number);

    let kind_of = |path: &str| {
        let (s, f) = model.find_field(path).expect(path);
        model.field(s, f).expect(path).kind.clone()
    };
    assert_eq!(
        kind_of("terminal.cursor_shape"),
        FieldKind::Choice(vec!["block".into(), "bar".into(), "underline".into()])
    );
    assert_eq!(kind_of("terminal.scrollback_lines"), FieldKind::Integer);
    assert_eq!(kind_of("terminal.exec_shells"), FieldKind::List);
    assert_eq!(kind_of("terminal.env"), FieldKind::Map);
    assert_eq!(kind_of("editor.font_family"), FieldKind::Text);
    assert!(kind_of("terminal.env").edited_in_json());
    assert!(!kind_of("editor.soft_wrap").edited_in_json());
    assert_eq!(model.field_count(), 14);
}

#[test]
fn a_section_without_page_metadata_is_titled_from_its_key_and_schema() {
    let schema = json!({
        "properties": {
            "watch_budget": {
                "description": "Limits on the feeds the app opens\non the cluster.",
                "type": "object",
                "properties": { "max_feeds": { "type": "integer" } }
            },
            "plain": {
                "$ref": "#/$defs/PlainContent"
            }
        },
        "$defs": {
            "PlainContent": {
                "description": "What one settings layer says about plain things.",
                "type": "object",
                "properties": { "on": { "type": "boolean" } }
            }
        }
    });
    let model = SettingsPageModel::from_schema(&schema);
    let budget = &model.sections()[model.section_index("watch_budget").expect("budget")];
    assert_eq!(budget.title, "Watch budget");
    // Hard line breaks of a doc comment become spaces.
    assert_eq!(
        budget.description.as_deref(),
        Some("Limits on the feeds the app opens on the cluster.")
    );
    // The content struct's "What one settings layer says" is schema wording, not page wording.
    let plain = &model.sections()[model.section_index("plain").expect("plain")];
    assert_eq!(plain.description, None);
    assert!(
        model.section_index(GENERAL_KEY).is_none(),
        "no top-level scalars"
    );
}

#[test]
fn a_broken_or_empty_schema_gives_an_empty_page() {
    assert!(
        SettingsPageModel::from_schema(&json!({}))
            .sections()
            .is_empty()
    );
    let cyclic = json!({
        "properties": { "a": { "$ref": "#/$defs/A" } },
        "$defs": { "A": { "$ref": "#/$defs/A" } }
    });
    let model = SettingsPageModel::from_schema(&cyclic);
    assert_eq!(
        model.field_count(),
        1,
        "a reference cycle stops, it does not hang"
    );
}

#[test]
fn titles_split_words_and_keep_acronyms() {
    assert_eq!(humanize("font_size"), "Font size");
    assert_eq!(humanize("ui_scale"), "UI scale");
    assert_eq!(humanize("terminal_cwd"), "Terminal CWD");
    assert_eq!(humanize("url"), "URL");
    assert_eq!(humanize("tls_server_name"), "TLS server name");
}

#[test]
fn the_real_schema_gives_a_section_per_object_and_every_field_a_title() {
    let mut store = oxikube_settings::SettingsStore::new(oxikube_assets::default_settings())
        .expect("the embedded defaults parse");
    store.set_user_settings("{}").expect("an empty user file");
    let model = SettingsPageModel::from_schema(&store.json_schema());
    assert_eq!(model.sections()[0].key, GENERAL_KEY);
    assert!(model.field_count() > 10, "{} fields", model.field_count());
    for section in model.sections() {
        assert!(!section.title.is_empty());
        for field in &section.fields {
            assert!(!field.title.is_empty(), "{} has no title", field.path);
        }
    }
}
