use super::*;
use serde_json::json;

fn parse(value: serde_json::Value) -> ThemeSelection {
    let content: ThemeSelectionContent = serde_json::from_value(value).unwrap();
    ThemeSettings::from_content(content).selection
}

#[test]
fn a_string_is_a_static_selection() {
    let selection = parse(json!("Ayu Dark"));
    assert_eq!(selection, ThemeSelection::Static("Ayu Dark".into()));
    assert_eq!(selection.name_for(Appearance::Light), "Ayu Dark");
    assert_eq!(selection.name_for(Appearance::Dark), "Ayu Dark");
}

#[test]
fn an_object_is_a_mode_with_light_and_dark_names() {
    let selection = parse(json!({ "mode": "system", "light": "Ayu Light", "dark": "Ayu Dark" }));
    assert_eq!(selection.name_for(Appearance::Light), "Ayu Light");
    assert_eq!(selection.name_for(Appearance::Dark), "Ayu Dark");

    let pinned = parse(json!({ "mode": "light", "light": "L", "dark": "D" }));
    assert_eq!(pinned.name_for(Appearance::Dark), "L");
    let pinned = parse(json!({ "mode": "dark", "light": "L", "dark": "D" }));
    assert_eq!(pinned.name_for(Appearance::Light), "D");
}

#[test]
fn omitted_parts_default_to_system_one_light_one_dark() {
    let empty = parse(json!({}));
    assert_eq!(empty, ThemeSelection::default());
    assert_eq!(empty.name_for(Appearance::Light), "One Light");
    assert_eq!(empty.name_for(Appearance::Dark), "One Dark");

    let only_dark = parse(json!({ "dark": "Gruvbox Dark" }));
    assert_eq!(only_dark.name_for(Appearance::Dark), "Gruvbox Dark");
    assert_eq!(only_dark.name_for(Appearance::Light), "One Light");
    assert_eq!(
        parse(json!({ "mode": "dark" })).name_for(Appearance::Light),
        "One Dark"
    );
}

#[test]
fn invalid_modes_and_types_do_not_deserialize() {
    assert!(serde_json::from_value::<ThemeSelectionContent>(json!({ "mode": "auto" })).is_err());
    assert!(serde_json::from_value::<ThemeSelectionContent>(json!(3)).is_err());
}

#[test]
fn content_round_trips_without_null_fields() {
    let content = ThemeSelectionContent::Dynamic {
        mode: Some(ThemeMode::Dark),
        light: None,
        dark: None,
    };
    assert_eq!(
        serde_json::to_value(&content).unwrap(),
        json!({ "mode": "dark" })
    );
    assert_eq!(
        serde_json::to_value(ThemeSelectionContent::Static("X".into())).unwrap(),
        json!("X")
    );
}
