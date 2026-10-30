//! Acceptance tests of the importer contract: every mapped key lands in its token, partial
//! themes fall back, players merge over the base, unmapped keys are counted, and a large file
//! imports quickly.

use super::*;
use crate::test_fixtures::import;
use gpui::Hsla;

/// A colour string for the `n`th key, distinct per `n` and never equal to an One colour by
/// accident (the alpha channel carries the index).
fn probe(n: usize) -> (String, Hsla) {
    let alpha = 1 + (n % 254);
    let text = format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        n % 251,
        (n * 7) % 251,
        (n * 13) % 251,
        alpha
    );
    let parsed = crate::color::parse_color(&text).expect("probe colour parses");
    (text, parsed)
}

fn family(style: &str) -> String {
    format!(
        r#"{{ "name": "T", "themes": [{{ "name": "T Dark", "appearance": "dark", "style": {style} }}] }}"#
    )
}

#[test]
fn every_mapped_key_sets_its_token() {
    let mut keys: Vec<&str> = table::mapped_keys().collect();
    keys.sort_unstable();
    assert!(keys.len() > 100, "the table maps the Zed v0.2.0 keys");
    for (index, key) in keys.iter().enumerate() {
        let (text, expected) = probe(index);
        let imported = import(&family(&format!(r#"{{ "{key}": "{text}" }}"#)));
        assert!(imported.report.is_clean(), "{key}: {:?}", imported.report);
        assert!(imported.report.unknown_keys.is_empty(), "{key} is mapped");
        let mut tokens = imported.family.themes[0].clone();
        let slot = table::color_slot(key).expect("mapped key has a slot");
        assert_eq!(*slot(&mut tokens), expected, "{key} -> its token");
    }
}

#[test]
fn only_background_and_text_leaves_every_other_token_at_the_fallback() {
    let imported = import(&family(
        r##"{ "background": "#101010", "text": "#fafafa" }"##,
    ));
    let tokens = &imported.family.themes[0];
    let base = ThemeTokens::fallback(Appearance::Dark);
    assert_eq!(
        tokens.colors.background,
        crate::color::parse_color("#101010").unwrap()
    );
    assert_eq!(
        tokens.colors.text,
        crate::color::parse_color("#fafafa").unwrap()
    );
    assert_eq!(tokens.colors.border, base.colors.border);
    assert_eq!(tokens.colors.text_muted, base.colors.text_muted);
    assert_eq!(tokens.editor, base.editor);
    assert_eq!(tokens.terminal.ansi, base.terminal.ansi);
    assert_eq!(tokens.vcs, base.vcs);
    assert_eq!(tokens.players, base.players);
    assert_eq!(tokens.accents, base.accents);
    assert_eq!(tokens.syntax, base.syntax);
    assert!(imported.report.is_clean());
}

#[test]
fn a_theme_with_three_players_still_has_eight() {
    let style = r##"{ "players": [
        { "cursor": "#111111", "background": "#111111", "selection": "#11111133" },
        { "cursor": "#222222", "background": "#222222", "selection": "#22222233" },
        { "cursor": "#333333", "background": "#333333", "selection": "#33333333" } ] }"##;
    let imported = import(&family(style));
    let players = &imported.family.themes[0].players;
    let base = &ThemeTokens::fallback(Appearance::Dark).players;
    assert_eq!(players.len(), base.len().max(3));
    assert_eq!(players.len(), 8);
    let c = |text| crate::color::parse_color(text).unwrap();
    assert_eq!(players[0].cursor, c("#111111"));
    assert_eq!(players[2].cursor, c("#333333"));
    assert_eq!(players[3..], base[3..], "the rest come from the base");
}

#[test]
fn players_beyond_the_base_are_appended() {
    let entries = (0..10)
        .map(|n| format!(r##"{{ "cursor": "#0000{n:02x}" }}"##))
        .collect::<Vec<_>>()
        .join(",");
    let imported = import(&family(&format!(r#"{{ "players": [{entries}] }}"#)));
    assert_eq!(imported.family.themes[0].players.len(), 10);
}

#[test]
fn random_key_subsets_apply_exactly_the_keys_given() {
    let keys: Vec<&str> = {
        let mut keys: Vec<_> = table::mapped_keys().collect();
        keys.sort_unstable();
        keys
    };
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for round in 0..40 {
        let chosen: Vec<(usize, &str)> = keys
            .iter()
            .copied()
            .enumerate()
            .filter(|_| next() % 4 == 0)
            .collect();
        let body = chosen
            .iter()
            .map(|(index, key)| format!(r#""{key}": "{}""#, probe(*index).0))
            .collect::<Vec<_>>()
            .join(",");
        let imported = import(&family(&format!("{{ {body} }}")));
        assert!(
            imported.report.is_clean(),
            "round {round}: {:?}",
            imported.report
        );
        let mut tokens = imported.family.themes[0].clone();
        for (index, key) in chosen {
            let slot = table::color_slot(key).expect("slot");
            assert_eq!(*slot(&mut tokens), probe(index).1, "round {round}: {key}");
        }
    }
}

#[test]
fn unmapped_keys_are_counted_once_across_themes() {
    let text = r##"{ "name": "U", "themes": [
        { "name": "A", "appearance": "dark", "style": { "x.one": "#fff", "x.two": "#fff", "text": "#fff" } },
        { "name": "B", "appearance": "light", "style": { "x.one": "#fff", "x.three": "#fff" },
          "oxikube": { "mystery": "#fff" } } ] }"##;
    let report = import(text).report;
    assert_eq!(report.unknown_keys.len(), 4, "{:?}", report.unknown_keys);
    assert!(report.unknown_keys.contains("oxikube.mystery"));
    assert!(report.is_clean());
}

#[test]
fn import_is_deterministic() {
    let a = import(crate::test_fixtures::GRUVBOX);
    let b = import(crate::test_fixtures::GRUVBOX);
    assert_eq!(a, b);
}

#[test]
fn a_large_theme_file_imports_quickly() {
    // ~200 KB: many themes of the full mapped key set.
    let keys: Vec<&str> = table::mapped_keys().collect();
    let style = keys
        .iter()
        .enumerate()
        .map(|(index, key)| format!(r#""{key}": "{}""#, probe(index).0))
        .collect::<Vec<_>>()
        .join(",");
    let themes = (0..(200_000 / (style.len() + 60)).max(1))
        .map(|n| format!(r#"{{ "name": "T{n}", "appearance": "dark", "style": {{ {style} }} }}"#))
        .collect::<Vec<_>>()
        .join(",");
    let text = format!(r#"{{ "name": "Big", "themes": [{themes}] }}"#);
    assert!(text.len() >= 150_000, "fixture is large: {}", text.len());
    let _warm = import_family(&text).expect("imports");
    let start = std::time::Instant::now();
    let imported = import_family(&text).expect("imports");
    let elapsed = start.elapsed();
    assert!(!imported.family.themes.is_empty());
    // Generous bound (debug build, loaded CI): the hot-reload path must feel instant.
    assert!(elapsed.as_millis() < 250, "200 KB import took {elapsed:?}");
}
