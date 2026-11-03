//! The control a schema node gets.

use super::model;
use crate::controls::{ControlSpec, NumberBounds};

fn control(path: &str) -> ControlSpec {
    let model = model();
    let (section, field) = model.find_field(path).expect("the fixture has the field");
    model
        .field(section, field)
        .expect("indexed")
        .control
        .clone()
}

#[test]
fn each_schema_type_gets_its_control() {
    assert_eq!(control("confirm_quit"), ControlSpec::Switch);
    assert_eq!(
        control("terminal.cursor_shape"),
        ControlSpec::Choice(vec!["block".into(), "bar".into()])
    );
    assert_eq!(control("terminal.exec_shells"), ControlSpec::List);
    assert_eq!(control("terminal.shell"), ControlSpec::Text);
    assert_eq!(control("editor.font_family"), ControlSpec::Font);
}

#[test]
fn numbers_carry_their_bounds() {
    assert_eq!(
        control("terminal.font_size"),
        ControlSpec::Number(NumberBounds {
            integer: false,
            min: Some(6.0),
            max: Some(72.0)
        })
    );
    assert_eq!(
        control("terminal.scrollback_lines"),
        ControlSpec::Number(NumberBounds {
            integer: true,
            min: Some(0.0),
            max: None
        })
    );
}

#[test]
fn a_colour_is_marked_by_format_or_by_the_hex_pattern() {
    assert_eq!(control("terminal.accent"), ControlSpec::Color);
    assert_eq!(control("editor.colour"), ControlSpec::Color);
}

#[test]
fn shapes_without_a_control_fall_back_to_json() {
    let spec = control("terminal.env");
    assert_eq!(spec, ControlSpec::Json);
    assert!(spec.is_json_only());
    assert!(!ControlSpec::Switch.is_json_only());
}

#[test]
fn only_switches_and_choices_write_without_a_pause() {
    assert!(ControlSpec::Switch.writes_immediately());
    assert!(ControlSpec::Choice(vec![]).writes_immediately());
    assert!(!ControlSpec::Text.writes_immediately());
    assert!(!ControlSpec::List.writes_immediately());
}

#[test]
fn the_real_schema_gets_the_expected_controls() {
    // The checked-in schema has every crate's settings; the test binary links only some.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../platform/oxikube_assets/assets/settings/settings.schema.json"
    );
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("the schema is checked in"))
            .expect("the schema is JSON");
    let model = crate::page::SettingsPageModel::from_schema(&schema);
    let control = |path: &str| {
        let (section, field) = model.find_field(path).unwrap_or_else(|| panic!("{path}"));
        model
            .field(section, field)
            .expect("indexed")
            .control
            .clone()
    };
    assert_eq!(control("read_only"), ControlSpec::Switch);
    assert_eq!(
        control("colour"),
        ControlSpec::Color,
        "ClusterColour is found by its pattern"
    );
    assert_eq!(control("terminal.font_family"), ControlSpec::Font);
    assert_eq!(control("editor.font_family"), ControlSpec::Font);
    assert!(matches!(
        control("terminal.font_size"),
        ControlSpec::Number(_)
    ));
    assert!(matches!(control("base_keymap"), ControlSpec::Choice(_)));
    assert_eq!(control("terminal.exec_shells"), ControlSpec::List);
    let editor_tabs = control("editor.tab_size");
    let ControlSpec::Number(bounds) = editor_tabs else {
        panic!("tab_size is a number: {editor_tabs:?}");
    };
    assert!(bounds.integer);
}
