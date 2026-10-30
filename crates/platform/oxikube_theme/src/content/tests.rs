use super::*;
use crate::test_fixtures::{AYU, GRUVBOX, ONE};

const COMMITTED: &str = include_str!("../../schema/theme-family.schema.json");

fn parse(text: &str) -> ThemeFamilyContent {
    let mut de = serde_json_lenient::Deserializer::from_str(text);
    ThemeFamilyContent::deserialize(&mut de).expect("fixture parses")
}

#[test]
fn bundled_fixtures_parse_into_the_typed_model() {
    for (text, name, themes) in [(ONE, "One", 2), (AYU, "Ayu", 3), (GRUVBOX, "Gruvbox", 6)] {
        let family = parse(text);
        assert_eq!(family.name, name);
        assert_eq!(family.themes.len(), themes, "{name}");
        assert!(family.themes.iter().all(|t| !t.style.0.is_empty()));
    }
}

#[test]
fn lenient_parser_accepts_comments_and_trailing_commas() {
    let family = parse(
        r##"{ // c
        "name": "X", "themes": [ { "name": "T", "appearance": "dark", "style": { "border": "#fff", }, }, ], }"##,
    );
    assert_eq!(family.themes[0].appearance, AppearanceContent::Dark);
    assert_eq!(
        crate::Appearance::from(AppearanceContent::Light),
        crate::Appearance::Light
    );
}

#[test]
fn unknown_style_keys_are_kept_for_the_unmapped_count() {
    let family = parse(
        r##"{ "name": "X", "themes": [ { "name": "T", "appearance": "light", "style": { "future.key": "#fff" } } ] }"##,
    );
    assert!(family.themes[0].style.0.contains_key("future.key"));
}

#[test]
fn schema_lists_every_mapped_key_and_the_oxikube_block() {
    let text = theme_family_schema_text();
    let schema: Value = serde_json::from_str(&text).expect("valid JSON");
    let style = &schema["$defs"]["ThemeStyleContent"]["properties"];
    for key in crate::import::table::mapped_keys() {
        assert!(style.get(key).is_some(), "missing {key}");
    }
    for key in ["players", "accents", "syntax"] {
        assert!(style.get(key).is_some(), "missing {key}");
    }
    let block = &schema["$defs"]["OxikubeContent"]["properties"];
    assert!(block.get("status.running").is_some());
    assert!(block.get("cluster.tab.8").is_some());
    assert!(block.get("log.source.10").is_some());
}

#[test]
fn committed_schema_is_current() {
    let generated = theme_family_schema_text();
    if std::env::var_os("OXIKUBE_UPDATE_THEME_SCHEMA").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/schema/theme-family.schema.json"
        );
        std::fs::write(path, &generated).expect("write schema");
        return;
    }
    assert_eq!(
        COMMITTED, generated,
        "schema/theme-family.schema.json is stale; run `OXIKUBE_UPDATE_THEME_SCHEMA=1 cargo test -p oxikube_theme --lib content::` and commit"
    );
}
