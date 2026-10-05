use super::*;
use crate::color::parse_color;
use crate::test_fixtures::{AYU, GRUVBOX, ONE, import};
use gpui::Hsla;

fn color(text: &str) -> Hsla {
    parse_color(text).unwrap()
}

fn theme<'a>(family: &'a ImportedFamily, name: &str) -> &'a ThemeTokens {
    family
        .family
        .themes
        .iter()
        .find(|theme| theme.name == name)
        .unwrap_or_else(|| panic!("no theme {name}"))
}

#[test]
fn bundled_one_imports_cleanly_and_leaves_no_placeholder() {
    let imported = import(ONE);
    assert_eq!(imported.family.name, "One");
    assert_eq!(imported.family.author, "Zed Industries");
    assert!(imported.report.is_clean(), "{:?}", imported.report);
    // Every key One uses is mapped (or one of players / accents / syntax).
    assert!(
        imported.report.unknown_keys.is_empty(),
        "unmapped: {:?}",
        imported.report.unknown_keys
    );

    let dark = theme(&imported, "One Dark");
    assert_eq!(dark.appearance, Appearance::Dark);
    assert_eq!(dark.colors.border, color("#464b57ff"));
    assert_eq!(dark.colors.text_accent, color("#74ade8ff"));
    assert_eq!(dark.editor.background, color("#282c33ff"));
    let light = theme(&imported, "One Light");
    assert_eq!(light.appearance, Appearance::Light);
    assert_eq!(light.colors.text_accent, color("#5c78e2ff"));
    assert_eq!(light.colors.background, color("#dcdcddff"));
}

#[test]
fn fallbacks_are_fully_populated() {
    use crate::tokens::placeholder_color;
    for appearance in [Appearance::Dark, Appearance::Light] {
        let tokens = ThemeTokens::fallback(appearance);
        let placeholder = placeholder_color();
        let rendered = format!("{tokens:?}");
        let marker = format!("{placeholder:?}");
        assert!(
            !rendered.contains(&marker),
            "{appearance:?} fallback still has a blank slot"
        );
        assert_eq!(
            tokens.name,
            if appearance.is_dark() {
                "One Dark"
            } else {
                "One Light"
            }
        );
        assert_eq!(tokens.players.len(), 8);
        assert!(tokens.syntax.style("keyword").is_some());
    }
}

#[test]
fn ayu_imports_with_expected_tokens() {
    let imported = import(AYU);
    assert_eq!(imported.family.name, "Ayu");
    let names: Vec<_> = imported
        .family
        .themes
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, ["Ayu Dark", "Ayu Light", "Ayu Mirage"]);
    assert!(imported.report.is_clean(), "{:?}", imported.report);

    let dark = theme(&imported, "Ayu Dark");
    assert_eq!(dark.appearance, Appearance::Dark);
    assert_eq!(dark.colors.background, color("#313337ff"));
    assert_eq!(dark.editor.background, color("#0d1016ff"));
    assert_eq!(dark.colors.text, color("#bfbdb6ff"));
    assert_eq!(dark.colors.text_accent, color("#5ac1feff"));
    assert_eq!(dark.colors.element_hover, color("#2d2f34ff"));
    assert_eq!(dark.terminal.ansi.red, color("#ef7177ff"));
    assert_eq!(dark.status.error.foreground, color("#ef7177ff"));
    assert_eq!(dark.status.success.foreground, color("#aad84cff"));
    assert_eq!(dark.status.warning.foreground, color("#feb454ff"));
    assert_eq!(
        dark.syntax.style("keyword").unwrap().color,
        Some(color("#ff8f3fff"))
    );
    assert_eq!(
        dark.syntax.style("comment").unwrap().color,
        Some(color("#5c6773ff"))
    );
    assert_eq!(dark.players.len(), 8);
    assert_eq!(dark.players[0].cursor, color("#5ac1feff"));
    assert_eq!(dark.players[0].selection, color("#5ac1fe3d"));
    assert_eq!(
        dark.colors.selection,
        color("#5ac1fe3d"),
        "derived from player 0"
    );

    let light = theme(&imported, "Ayu Light");
    assert_eq!(light.appearance, Appearance::Light);
    assert_eq!(light.editor.background, color("#fcfcfcff"));
    assert_eq!(
        light.syntax.style("keyword").unwrap().color,
        Some(color("#fa8d3eff"))
    );
    assert_eq!(theme(&imported, "Ayu Mirage").appearance, Appearance::Dark);
}

#[test]
fn gruvbox_imports_with_expected_tokens() {
    let imported = import(GRUVBOX);
    assert_eq!(imported.family.themes.len(), 6);
    assert!(imported.report.is_clean(), "{:?}", imported.report);

    let hard = theme(&imported, "Gruvbox Dark Hard");
    assert_eq!(hard.editor.background, color("#1d2021ff"));
    assert_eq!(hard.colors.background, color("#4c4642ff"));
    assert_eq!(hard.colors.text, color("#fbf1c7ff"));
    assert_eq!(hard.colors.scrollbar_thumb_active, color("#83a598ac"));
    assert_eq!(hard.status.error.foreground, color("#fb4a35ff"));
    assert_eq!(hard.vcs.added, color("#b7bb26ff"));
    assert_eq!(
        hard.syntax.style("keyword").unwrap().color,
        Some(color("#fb4833ff"))
    );
    assert_eq!(hard.accents.len(), 7);
    assert_eq!(hard.accents[0], color("#cc241dff"));

    let light = theme(&imported, "Gruvbox Light");
    assert_eq!(light.appearance, Appearance::Light);
    assert_eq!(light.editor.background, color("#fbf1c7ff"));
    assert_eq!(light.colors.text, color("#282828ff"));
}

#[test]
fn missing_keys_fall_back_to_the_bundled_theme_of_the_same_appearance() {
    // Ayu has no `version_control.*`: the dark theme inherits One Dark's, the light One Light's.
    let imported = import(AYU);
    assert_eq!(
        theme(&imported, "Ayu Dark").vcs,
        ThemeTokens::fallback(Appearance::Dark).vcs
    );
    assert_eq!(
        theme(&imported, "Ayu Light").vcs,
        ThemeTokens::fallback(Appearance::Light).vcs
    );

    // A theme that sets nothing is the fallback, renamed.
    let text = r##"{ "name": "Bare", "themes": [{ "name": "Bare Dark", "appearance": "dark", "style": {} }] }"##;
    let bare = import(text);
    let tokens = &bare.family.themes[0];
    let mut expected = ThemeTokens::fallback(Appearance::Dark).clone();
    expected.name = "Bare Dark".into();
    assert_eq!(tokens, &expected);
    assert!(bare.report.is_clean());
}

#[test]
fn partial_themes_keep_what_they_set() {
    let text = r##"{ "name": "P", "themes": [{ "name": "P", "appearance": "light",
        "style": { "text": "#112233", "syntax": { "keyword": { "color": "#445566", "font_style": "italic" } } } }] }"##;
    let imported = import(text);
    let tokens = &imported.family.themes[0];
    assert_eq!(tokens.colors.text, color("#112233ff"));
    assert_eq!(
        tokens.colors.border,
        ThemeTokens::fallback(Appearance::Light).colors.border
    );
    let keyword = tokens.syntax.style("keyword").unwrap();
    assert_eq!(keyword.color, Some(color("#445566ff")));
    assert_eq!(keyword.font_style, Some(crate::tokens::FontStyle::Italic));
    // Entries the file does not mention keep the fallback's style.
    assert_eq!(
        tokens.syntax.style("comment"),
        ThemeTokens::fallback(Appearance::Light)
            .syntax
            .style("comment")
    );
}

#[test]
fn invalid_values_are_reported_and_keep_the_fallback() {
    let text = r##"{ "name": "Bad", "themes": [{ "name": "Bad Dark", "appearance": "dark", "style": {
        "text": "red",
        "border": 12,
        "background": "#3b414d",
        "text.muted": null,
        "players": "nope",
        "accents": ["#fff", "oops"],
        "syntax": { "keyword": { "color": "#xyz" }, "string": 4 }
    } }] }"##;
    let imported = import(text);
    let tokens = &imported.family.themes[0];
    let fallback = ThemeTokens::fallback(Appearance::Dark);
    assert_eq!(
        tokens.colors.text, fallback.colors.text,
        "invalid colour keeps the fallback"
    );
    assert_eq!(tokens.colors.border, fallback.colors.border);
    assert_eq!(
        tokens.colors.background,
        color("#3b414dff"),
        "valid keys still apply"
    );
    assert_eq!(
        tokens.colors.text_muted, fallback.colors.text_muted,
        "null is unspecified"
    );
    assert_eq!(tokens.accents, vec![color("#ffffffff")]);

    let problems: Vec<String> = imported
        .report
        .diagnostics
        .iter()
        .map(ToString::to_string)
        .collect();
    let has = |needle: &str| problems.iter().any(|p| p.contains(needle));
    assert!(has("`text` is not a colour"), "{problems:?}");
    assert!(has("`border` should be a colour string"), "{problems:?}");
    assert!(has("`players` should be an array"), "{problems:?}");
    assert!(has("`accents[1]`"), "{problems:?}");
    assert!(has("`syntax.keyword.color`"), "{problems:?}");
    assert!(has("`syntax.string` should be an object"), "{problems:?}");
    assert!(!has("text.muted"), "null is not a problem: {problems:?}");
    assert!(!imported.report.is_clean());
}

#[test]
fn invalid_themes_are_skipped_not_fatal() {
    let text = r##"{ "name": "Mixed", "themes": [
        { "name": "Fine", "appearance": "dark", "style": {} },
        { "name": "Odd", "appearance": "sepia", "style": {} },
        { "appearance": "dark" },
        42
    ] }"##;
    let imported = import(text);
    assert_eq!(imported.family.themes.len(), 1);
    assert_eq!(imported.report.diagnostics.len(), 3);
    assert!(matches!(
        imported.report.diagnostics[0],
        ImportDiagnostic::SkippedTheme { index: 1, .. }
    ));
}

#[test]
fn unknown_keys_are_ignored_and_listed() {
    let text = r##"{ "name": "U", "themes": [{ "name": "U", "appearance": "dark", "style": {
        "panel.indent_guide": "#ffffff", "text": "#010203" } }] }"##;
    let imported = import(text);
    assert!(imported.report.is_clean(), "unknown keys are not problems");
    assert!(imported.report.unknown_keys.contains("panel.indent_guide"));
    assert_eq!(imported.family.themes[0].colors.text, color("#010203ff"));
}

#[test]
fn lenient_json_and_schema_versions() {
    let text = r##"// a comment
    {
      "$schema": "https://zed.dev/schema/themes/v0.2.0.json",
      "name": "L",
      "themes": [ { "name": "L", "appearance": "dark", "style": { "text": "#010203", }, }, ],
    }"##;
    let imported = import(text);
    assert!(imported.report.is_clean(), "{:?}", imported.report);
    assert_eq!(imported.family.themes[0].colors.text, color("#010203ff"));

    let other = r##"{ "$schema": "https://zed.dev/schema/themes/v9.9.9.json", "name": "S", "themes": [] }"##;
    assert!(matches!(
        import(other).report.diagnostics[0],
        ImportDiagnostic::SchemaVersion { .. }
    ));
}

#[test]
fn files_that_are_not_theme_families_are_errors() {
    assert!(matches!(import_family("{"), Err(ImportError::Json(_))));
    assert_eq!(import_family("[]"), Err(ImportError::NotAThemeFamily));
    assert_eq!(
        import_family(r#"{ "name": "x" }"#),
        Err(ImportError::NotAThemeFamily)
    );
}

#[test]
fn oxikube_status_colours_default_from_the_themes_own_status_colours() {
    let imported = import(AYU);
    let dark = theme(&imported, "Ayu Dark");
    assert_eq!(dark.oxikube.status_running, dark.status.success.foreground);
    assert_eq!(dark.oxikube.status_pending, dark.status.warning.foreground);
    assert_eq!(dark.oxikube.status_failed, dark.status.error.foreground);
    assert_eq!(dark.oxikube.status_succeeded, dark.status.info.foreground);
    assert_eq!(
        dark.oxikube.status_terminating,
        dark.status.hidden.foreground
    );
    assert_eq!(dark.oxikube.status_unknown, dark.colors.text_muted);
    // The cluster tab palette cycles through the player cursors.
    assert_eq!(dark.oxikube.cluster_tabs[0], dark.players[0].cursor);
    assert_eq!(dark.oxikube.cluster_tabs[7], dark.players[7].cursor);
}

#[test]
fn oxikube_block_overrides_defaults() {
    let text = r##"{ "name": "O", "themes": [{ "name": "O", "appearance": "dark", "style": {},
        "oxikube": { "status.running": "#00ff00", "status.failed": "#ff0000cc",
                     "cluster.tab.2": "#123456", "cluster.tab.9": "#ffffff", "status.bogus": "#fff",
                     "status.pending": "nope" } }] }"##;
    let imported = import(text);
    let tokens = &imported.family.themes[0];
    let default = import(r##"{ "themes": [{ "name": "D", "appearance": "dark" }] }"##)
        .family
        .themes[0]
        .oxikube;
    assert_eq!(tokens.oxikube.status_running, color("#00ff00ff"));
    assert_eq!(tokens.oxikube.status_failed, color("#ff0000cc"));
    assert_eq!(tokens.oxikube.cluster_tabs[1], color("#123456ff"));
    assert_eq!(tokens.oxikube.cluster_tabs[0], default.cluster_tabs[0]);
    assert_eq!(
        tokens.oxikube.status_pending, default.status_pending,
        "invalid override keeps the default"
    );
    assert_eq!(tokens.oxikube.status_succeeded, default.status_succeeded);
    assert!(
        imported
            .report
            .unknown_keys
            .contains("oxikube.status.bogus")
    );
    assert!(
        imported
            .report
            .unknown_keys
            .contains("oxikube.cluster.tab.9")
    );
    assert_eq!(
        imported.report.diagnostics.len(),
        1,
        "{:?}",
        imported.report.diagnostics
    );
}

#[test]
fn on_accent_contrasts_with_the_accent() {
    let on_accent = |accent: &str| {
        let text = format!(
            r##"{{ "themes": [{{ "name": "T", "appearance": "dark", "style": {{ "text.accent": "{accent}" }} }}] }}"##
        );
        import(&text).family.themes[0].colors.on_accent
    };
    assert!(on_accent("#74ade8").l < 0.2, "light accent gets dark ink");
    assert_eq!(on_accent("#0b6678").l, 1.0, "dark accent gets white");
}

#[test]
fn every_mapped_key_has_a_unique_slot() {
    // Setting each mapped key to a distinct colour must land in distinct fields: a duplicated
    // path in the table would make two keys collide.
    let mut tokens = ThemeTokens::placeholder(Appearance::Dark);
    let keys: Vec<&str> = table::mapped_keys().collect();
    for (index, key) in keys.iter().enumerate() {
        let slot = table::color_slot(key).unwrap();
        *slot(&mut tokens) =
            gpui::hsla(0.0, 0.0, 0.0, 0.001 * (index + 1) as f32 / 1000.0 + 0.0001);
    }
    let rendered = format!("{tokens:?}");
    for (index, _) in keys.iter().enumerate() {
        let alpha = 0.001 * (index + 1) as f32 / 1000.0 + 0.0001;
        assert!(
            rendered.contains(&format!("a: {alpha:?}")),
            "slot {index} was overwritten by another key"
        );
    }
}
