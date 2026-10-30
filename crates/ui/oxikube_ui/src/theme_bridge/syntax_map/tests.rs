//! One test per table entry (the E22 risk note: a mapping table needs a test per entry).

use super::{EDITOR_CAPTURES, editor_capture, resolve};
use crate::theme_bridge::config;
use gpui::hsla;
use oxikube_theme::tokens::{SyntaxStyle, SyntaxTheme};
use oxikube_theme::{Appearance, ThemeTokens};

fn style(hue: f32) -> SyntaxStyle {
    SyntaxStyle {
        color: Some(hsla(hue, 1., 0.5, 1.)),
        ..SyntaxStyle::default()
    }
}

fn only(name: &str, hue: f32) -> SyntaxTheme {
    let mut theme = SyntaxTheme::default();
    theme.styles.insert(name.to_owned(), style(hue));
    theme
}

#[test]
fn the_table_has_no_duplicate_names() {
    for (index, capture) in EDITOR_CAPTURES.iter().enumerate() {
        assert!(
            EDITOR_CAPTURES[..index]
                .iter()
                .all(|c| c.name != capture.name),
            "duplicate {}",
            capture.name
        );
    }
}

#[test]
fn every_entry_resolves_its_own_theme_key() {
    for capture in &EDITOR_CAPTURES {
        let theme = only(capture.name, 0.3);
        assert_eq!(
            resolve(&theme, capture),
            Some(&style(0.3)),
            "{}",
            capture.name
        );
    }
}

#[test]
fn every_entry_without_coverage_is_plain_text() {
    let empty = SyntaxTheme::default();
    for capture in &EDITOR_CAPTURES {
        assert_eq!(resolve(&empty, capture), None, "{}", capture.name);
    }
}

#[test]
fn every_dotted_entry_falls_back_to_its_ancestor() {
    for capture in EDITOR_CAPTURES.iter().filter(|c| c.name.contains('.')) {
        let (parent, _) = capture.name.rsplit_once('.').expect("dotted");
        let theme = only(parent, 0.6);
        assert_eq!(
            resolve(&theme, capture),
            Some(&style(0.6)),
            "{}",
            capture.name
        );
    }
}

#[test]
fn every_alias_stands_in_for_its_capture() {
    for capture in EDITOR_CAPTURES.iter().filter(|c| c.alias.is_some()) {
        let alias = capture.alias.expect("filtered");
        let theme = only(alias, 0.9);
        assert_eq!(
            resolve(&theme, capture),
            Some(&style(0.9)),
            "{}",
            capture.name
        );
    }
}

#[test]
fn an_alias_is_itself_an_editor_capture() {
    for capture in EDITOR_CAPTURES.iter().filter_map(|c| c.alias) {
        assert_eq!(editor_capture(capture), Some(capture), "{capture}");
    }
}

#[test]
fn the_themes_own_entry_beats_the_alias() {
    let mut theme = only("constant", 0.1);
    theme.styles.insert("boolean".into(), style(0.2));
    let boolean = EDITOR_CAPTURES
        .iter()
        .find(|c| c.name == "boolean")
        .expect("entry");
    assert_eq!(resolve(&theme, boolean), Some(&style(0.2)));
}

/// The captures the JSON (gpui-component) and YAML (tree-sitter-yaml 0.7) highlight queries emit,
/// and the editor capture each is painted as.
const QUERY_CAPTURES: [(&str, Option<&str>); 14] = [
    ("attribute", Some("attribute")),
    ("boolean", Some("boolean")),
    ("comment", Some("comment")),
    ("constant.builtin", Some("constant")),
    ("label", Some("label")),
    ("number", Some("number")),
    ("property", Some("property")),
    ("punctuation", Some("punctuation")),
    ("punctuation.bracket", Some("punctuation.bracket")),
    ("punctuation.delimiter", Some("punctuation.delimiter")),
    ("punctuation.special", Some("punctuation.special")),
    ("string", Some("string")),
    ("string.escape", Some("string.escape")),
    ("type", Some("type")),
];

#[test]
fn every_yaml_and_json_query_capture_maps() {
    for (raw, expected) in QUERY_CAPTURES {
        assert_eq!(editor_capture(raw), expected, "{raw}");
    }
}

#[test]
fn an_unknown_capture_is_plain_text() {
    assert_eq!(editor_capture("nonsense"), None);
    assert_eq!(editor_capture("nonsense.deeper"), None);
    assert_eq!(editor_capture(""), None);
}

#[test]
fn the_highlight_section_carries_the_resolved_aliases() {
    let mut theme = ThemeTokens::fallback(Appearance::Dark).clone();
    theme.syntax = only("constant", 0.4);
    let highlight = config::highlight(&theme);
    let syntax = &highlight["syntax"];
    for name in ["constant", "boolean", "number"] {
        assert!(syntax[name]["color"].is_string(), "{name}");
    }
    assert!(syntax["comment"].is_null(), "uncovered stays plain text");
}
