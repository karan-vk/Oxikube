//! Conformance of the Zed theme importer against real-world theme files (E22-S12).
//!
//! The fixtures under `tests/fixtures/zed-themes/` are MIT-licensed theme files from Zed theme
//! extensions (one row each in its `LICENSES.md`), plus Zed's own Ayu, Gruvbox and One one
//! directory up. The tests run offline and need no display.
//!
//! The unmapped-key counts live in `tests/fixtures/zed-themes/import-report.golden`. When the
//! importer or the token set changes them on purpose, regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p oxikube_theme --test conformance`.

use gpui::hsla;
use oxikube_theme::{
    Appearance, ImportError, ImportedFamily, ThemeRegistry, ThemeSource, ThemeTokens, import_family,
};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const MIN_FAMILIES: usize = 20;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The `.json` files directly inside `dir`, by name.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("fixture has a name")
        .to_string_lossy()
        .into_owned()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// The licence-checked real-world families: the extension themes plus Zed's Ayu, Gruvbox, One.
fn real_families() -> Vec<PathBuf> {
    let mut files = json_files(&fixtures().join("zed-themes"));
    for name in ["ayu.json", "gruvbox.json", "one.json"] {
        files.push(fixtures().join(name));
    }
    files
}

fn edge_files() -> Vec<PathBuf> {
    json_files(&fixtures().join("zed-themes/edge"))
}

fn import(path: &Path) -> ImportedFamily {
    import_family(&read(path)).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// The magenta-at-odd-alpha colour the importer's blank slate is filled with. A leftover would
/// mean a token the table never sets (see `ThemeTokens::placeholder`).
fn placeholder_marker() -> String {
    format!("{:?}", hsla(0.8333, 1.0, 0.5, 0.123))
}

fn assert_required_tokens(tokens: &ThemeTokens, source: &str) {
    let rendered = format!("{tokens:?}");
    assert!(
        !rendered.contains(&placeholder_marker()),
        "{source}: {} has an unset token",
        tokens.name
    );
    assert!(!tokens.name.is_empty(), "{source}: unnamed theme");
    assert!(
        !tokens.players.is_empty(),
        "{source}: {} has no players",
        tokens.name
    );
    assert!(
        !tokens.syntax.styles.is_empty(),
        "{source}: {} has no syntax styles",
        tokens.name
    );
}

#[test]
fn at_least_twenty_real_families_import_with_every_token_set() {
    let files = real_families();
    assert!(
        files.len() >= MIN_FAMILIES,
        "only {} real families, need {MIN_FAMILIES}",
        files.len()
    );
    let mut theme_count = 0;
    for path in &files {
        let source = file_name(path);
        let imported = import(path);
        assert!(!imported.family.themes.is_empty(), "{source}: no themes");
        for theme in &imported.family.themes {
            assert_required_tokens(theme, &source);
            theme_count += 1;
        }
    }
    assert!(theme_count >= MIN_FAMILIES, "only {theme_count} themes");
}

#[test]
fn real_themes_set_their_own_colours() {
    // A file that maps to nothing would silently be the fallback: its colours must differ
    // from the One fallback of the same appearance (One itself excepted).
    for path in real_families() {
        let source = file_name(&path);
        if source == "one.json" {
            continue;
        }
        for theme in import(&path).family.themes {
            let fallback = ThemeTokens::fallback(theme.appearance);
            // Compared as text: a theme may share one colour with One (base16 One Light does).
            let own = format!("{:?}{:?}{:?}", theme.colors, theme.editor, theme.syntax);
            let base = format!(
                "{:?}{:?}{:?}",
                fallback.colors, fallback.editor, fallback.syntax
            );
            assert_ne!(own, base, "{source}: {} equals the fallback", theme.name);
        }
    }
}

#[test]
fn bundled_families_import_cleanly_and_are_counted() {
    let bundled = oxikube_assets::BUNDLED_THEME_FAMILIES;
    assert!(bundled.len() >= 4, "One, Oxikube, Ayu and Gruvbox expected");
    for file in bundled {
        let imported =
            import_family(file.json).unwrap_or_else(|err| panic!("{}: {err}", file.file));
        assert!(
            imported.report.is_clean(),
            "{}: {:?}",
            file.file,
            imported.report
        );
        for theme in &imported.family.themes {
            assert_required_tokens(theme, file.file);
        }
    }
}

#[test]
fn every_family_has_a_detected_appearance_and_the_registry_accepts_it() {
    let mut registry = ThemeRegistry::default();
    let mut expected = Vec::new();
    for path in real_families().into_iter().chain(edge_files()) {
        let imported = import(&path);
        for theme in &imported.family.themes {
            // Later files win on a duplicate name; the expectation follows the same rule.
            expected.retain(|(name, _): &(String, Appearance)| name != &theme.name);
            expected.push((theme.name.clone(), theme.appearance));
        }
        registry.insert_bundled(imported.family);
    }
    let listed = registry.list();
    assert_eq!(listed.len(), expected.len());
    for (name, appearance) in expected {
        let meta = listed
            .iter()
            .find(|meta| meta.name == name)
            .unwrap_or_else(|| panic!("registry lost {name}"));
        assert_eq!(meta.appearance, appearance, "{name}");
        assert_eq!(meta.source, ThemeSource::Bundled, "{name}");
        let tokens = registry.get(&name).expect("registered theme resolves");
        assert_eq!(tokens.appearance, appearance, "{name}");
    }
    // Both appearances occur in the sample set.
    assert!(listed.iter().any(|m| m.appearance == Appearance::Dark));
    assert!(listed.iter().any(|m| m.appearance == Appearance::Light));
}

#[test]
fn every_fixture_has_a_licence_row() {
    let dir = fixtures().join("zed-themes");
    let licences = read(&dir.join("LICENSES.md"));
    for path in json_files(&dir) {
        let row = format!("| `{}` |", file_name(&path));
        assert!(
            licences.contains(&row),
            "{} has no row in zed-themes/LICENSES.md",
            file_name(&path)
        );
    }
    for line in licences.lines().filter(|l| l.starts_with("| `")) {
        let name = line.split('`').nth(1).expect("file name in a row");
        assert!(dir.join(name).is_file(), "LICENSES.md lists missing {name}");
        assert!(
            line.contains("| MIT |") || line.contains("| Apache-2.0 |"),
            "{line}"
        );
        assert!(line.contains("https://github.com/"), "{line}");
    }
    let zed = read(&fixtures().join("LICENSES.md"));
    for family in ["Ayu Dark", "Gruvbox Dark", "One Dark"] {
        assert!(zed.contains(family), "fixtures/LICENSES.md lacks {family}");
    }
}

// Edge cases.

fn edge(name: &str) -> ImportedFamily {
    import(&fixtures().join("zed-themes/edge").join(name))
}

#[test]
fn comments_and_trailing_commas_are_accepted() {
    let imported = edge("comments-trailing-commas.json");
    assert!(imported.report.is_clean(), "{:?}", imported.report);
    let theme = &imported.family.themes[0];
    // Players merge over the fallback's by index, so the one given replaces the first.
    assert_eq!(theme.players[0].cursor, hsla(32.0 / 360.0, 1.0, 0.5, 1.0));
    assert!(theme.syntax.style("keyword").is_some());
    assert_required_tokens(theme, "comments-trailing-commas.json");
}

#[test]
fn a_theme_with_a_handful_of_keys_falls_back_for_the_rest() {
    let imported = edge("few-keys.json");
    assert!(imported.report.is_clean(), "{:?}", imported.report);
    assert_eq!(imported.family.themes.len(), 4);
    for theme in &imported.family.themes {
        assert_required_tokens(theme, "few-keys.json");
        let fallback = ThemeTokens::fallback(theme.appearance);
        assert_eq!(theme.editor.background, fallback.editor.background);
    }
    assert_ne!(
        imported.family.themes[0].colors.background,
        ThemeTokens::fallback(Appearance::Dark).colors.background
    );
}

#[test]
fn many_players_are_all_kept() {
    let imported = edge("many-players.json");
    assert!(imported.report.is_clean(), "{:?}", imported.report);
    let theme = &imported.family.themes[0];
    assert_eq!(theme.players.len(), 40);
    assert_eq!(theme.accents.len(), 24);
}

#[test]
fn unknown_keys_are_counted_not_fatal() {
    let imported = edge("unknown-keys.json");
    assert!(imported.report.is_clean(), "{:?}", imported.report);
    assert!(imported.report.unknown_keys.contains("future.surface.glow"));
    assert!(imported.report.unknown_keys.contains("another.future.key"));
    assert_required_tokens(&imported.family.themes[0], "unknown-keys.json");
}

#[test]
fn syntax_font_style_and_weight_are_read() {
    use oxikube_theme::tokens::FontStyle;
    let imported = edge("font-style-weight.json");
    assert!(imported.report.is_clean(), "{:?}", imported.report);
    let syntax = &imported.family.themes[0].syntax;
    let style = |name: &str| *syntax.style(name).unwrap_or_else(|| panic!("no {name}"));
    assert_eq!(style("comment").font_style, Some(FontStyle::Italic));
    assert_eq!(style("keyword").font_weight, Some(700.0));
    assert_eq!(style("function").font_style, Some(FontStyle::Oblique));
    assert_eq!(style("string").font_style, Some(FontStyle::Normal));
}

// Truncation fuzz-lite.

/// Every prefix of `text` (at char boundaries, every `step`th byte) either imports or fails with
/// a typed error; it never panics.
fn assert_truncations_are_typed(text: &str, step: usize) {
    for end in (0..text.len()).step_by(step) {
        if !text.is_char_boundary(end) {
            continue;
        }
        match import_family(&text[..end]) {
            Ok(_) | Err(ImportError::Json(_) | ImportError::NotAThemeFamily) => {}
        }
    }
}

#[test]
fn truncated_files_give_typed_errors_not_panics() {
    for path in edge_files() {
        assert_truncations_are_typed(&read(&path), 1);
    }
    let one = read(&fixtures().join("one.json"));
    assert_truncations_are_typed(&one, 7);
    // Cut mid-file: definitely not valid JSON.
    assert!(matches!(
        import_family(&one[..one.len() / 2]),
        Err(ImportError::Json(_))
    ));
    assert!(matches!(import_family(""), Err(ImportError::Json(_))));
    assert!(matches!(
        import_family("[]"),
        Err(ImportError::NotAThemeFamily)
    ));
}

// Unmapped-key counts.

fn report_line(label: &str, imported: &ImportedFamily) -> String {
    let mut line = String::new();
    write!(
        line,
        "{label}\tthemes={}\tdiagnostics={}\tunknown_keys={}",
        imported.family.themes.len(),
        imported.report.diagnostics.len(),
        imported.report.unknown_keys.len()
    )
    .expect("writing to a String");
    line
}

#[test]
fn import_report_counts_match_the_golden_file() {
    let mut actual = String::from(
        "# file, themes imported, diagnostics, distinct unmapped keys.\n\
         # Regenerate with UPDATE_GOLDEN=1 cargo test -p oxikube_theme --test conformance\n",
    );
    for path in real_families().into_iter().chain(edge_files()) {
        let relative = path
            .strip_prefix(fixtures())
            .expect("fixture is under fixtures/")
            .to_string_lossy()
            .into_owned();
        actual.push_str(&report_line(&relative, &import(&path)));
        actual.push('\n');
    }
    for file in oxikube_assets::BUNDLED_THEME_FAMILIES {
        let imported = import_family(file.json).expect("bundled family imports");
        actual.push_str(&report_line(&format!("bundled/{}", file.file), &imported));
        actual.push('\n');
    }

    let golden = fixtures().join("zed-themes/import-report.golden");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &actual).expect("write golden");
        return;
    }
    assert_eq!(
        read(&golden),
        actual,
        "import report changed; if intended, rerun with UPDATE_GOLDEN=1"
    );
}
