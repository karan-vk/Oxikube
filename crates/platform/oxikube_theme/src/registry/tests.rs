use super::*;
use crate::appearance::ThemeMode;
use crate::settings::ThemeSelection;
use crate::test_fixtures::{AYU, GRUVBOX};
use std::path::PathBuf;

fn scan_of(files: &[(&str, &str)]) -> UserThemes {
    UserThemes {
        families: files
            .iter()
            .map(|(name, text)| (PathBuf::from(name), import_family(text).unwrap().family))
            .collect(),
        problems: Vec::new(),
    }
}

fn dynamic(mode: ThemeMode, light: &str, dark: &str) -> ThemeSelection {
    ThemeSelection::Dynamic {
        mode,
        light: light.into(),
        dark: dark.into(),
    }
}

#[test]
fn bundled_registry_has_one_dark_and_one_light() {
    let registry = ThemeRegistry::with_bundled();
    assert_eq!(
        registry.list(),
        vec![
            ThemeMeta {
                name: "One Dark".into(),
                appearance: Appearance::Dark,
                source: ThemeSource::Bundled
            },
            ThemeMeta {
                name: "One Light".into(),
                appearance: Appearance::Light,
                source: ThemeSource::Bundled
            },
        ]
    );
    assert_eq!(registry.default_for(Appearance::Dark).name, "One Dark");
    assert_eq!(registry.default_for(Appearance::Light).name, "One Light");
    assert!(registry.get("Nope").is_none());
}

#[test]
fn user_themes_are_listed_sorted_and_replaced_wholesale() {
    let mut registry = ThemeRegistry::with_bundled();
    registry.replace_user_themes(scan_of(&[("ayu.json", AYU), ("gruvbox.json", GRUVBOX)]));
    let names = registry.names();
    assert_eq!(names.len(), 2 + 3 + 6);
    let mut sorted = names.clone();
    sorted.sort_by_key(|name| name.to_lowercase());
    assert_eq!(names, sorted);
    assert!(registry.get("Ayu Mirage").is_some());
    assert!(registry.list().iter().any(|m| m.name == "Gruvbox Light"
        && m.source == ThemeSource::User
        && m.appearance == Appearance::Light));

    // A rescan without gruvbox drops it again.
    registry.replace_user_themes(scan_of(&[("ayu.json", AYU)]));
    assert!(registry.get("Gruvbox Light").is_none());
    assert!(registry.get("Ayu Dark").is_some());
    registry.replace_user_themes(UserThemes::default());
    assert_eq!(registry.names(), ["One Dark", "One Light"]);
}

#[test]
fn a_user_theme_shadows_a_bundled_one_of_the_same_name() {
    let mut registry = ThemeRegistry::with_bundled();
    let text = r##"{ "name": "Mine", "themes": [{ "name": "One Dark", "appearance": "dark", "style": { "text": "#010203" } }] }"##;
    registry.replace_user_themes(scan_of(&[("mine.json", text)]));
    assert_eq!(registry.names(), ["One Dark", "One Light"], "listed once");
    let theme = registry.get("One Dark").unwrap();
    assert_eq!(
        theme.colors.text,
        crate::color::parse_color("#010203").unwrap()
    );
    assert_eq!(registry.list()[0].source, ThemeSource::User);
    // The bundled default for the appearance is still the bundled theme.
    assert_ne!(
        registry.default_for(Appearance::Dark).colors.text,
        theme.colors.text
    );
}

#[test]
fn duplicate_names_across_user_files_keep_the_first() {
    let mut registry = ThemeRegistry::with_bundled();
    let first = r##"{ "themes": [{ "name": "Dup", "appearance": "dark", "style": { "text": "#111111" } }] }"##;
    let second = r##"{ "themes": [{ "name": "Dup", "appearance": "dark", "style": { "text": "#222222" } }] }"##;
    registry.replace_user_themes(scan_of(&[("a.json", first), ("b.json", second)]));
    assert_eq!(
        registry.get("Dup").unwrap().colors.text,
        crate::color::parse_color("#111111").unwrap()
    );
    let problems = registry.problems();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].path, PathBuf::from("b.json"));
    assert!(problems[0].message.contains("duplicate theme name"));
}

#[test]
fn selection_resolves_by_name_or_by_mode_and_system_appearance() {
    let mut registry = ThemeRegistry::with_bundled();
    registry.replace_user_themes(scan_of(&[("ayu.json", AYU)]));

    // A plain name ignores the system appearance.
    let ayu = ThemeSelection::Static("Ayu Light".into());
    assert_eq!(registry.resolve(&ayu, Appearance::Dark).name, "Ayu Light");
    assert_eq!(registry.resolve(&ayu, Appearance::Light).name, "Ayu Light");

    // system: follow the (fake) OS.
    let follow = dynamic(ThemeMode::System, "Ayu Light", "Ayu Dark");
    assert_eq!(
        registry.resolve(&follow, Appearance::Light).name,
        "Ayu Light"
    );
    assert_eq!(registry.resolve(&follow, Appearance::Dark).name, "Ayu Dark");

    // light / dark: pinned regardless of the OS.
    let pinned_light = dynamic(ThemeMode::Light, "Ayu Light", "Ayu Dark");
    assert_eq!(
        registry.resolve(&pinned_light, Appearance::Dark).name,
        "Ayu Light"
    );
    let pinned_dark = dynamic(ThemeMode::Dark, "Ayu Light", "Ayu Dark");
    assert_eq!(
        registry.resolve(&pinned_dark, Appearance::Light).name,
        "Ayu Dark"
    );
}

#[test]
fn unknown_names_fall_back_to_the_default_of_the_wanted_appearance() {
    let registry = ThemeRegistry::with_bundled();
    let missing = dynamic(ThemeMode::System, "Nope Light", "Nope Dark");
    assert_eq!(
        registry.resolve(&missing, Appearance::Light).name,
        "One Light"
    );
    assert_eq!(
        registry.resolve(&missing, Appearance::Dark).name,
        "One Dark"
    );
    let pinned = dynamic(ThemeMode::Dark, "x", "y");
    assert_eq!(
        registry.resolve(&pinned, Appearance::Light).name,
        "One Dark"
    );
    let missing_static = ThemeSelection::Static("Nope".into());
    assert_eq!(
        registry.resolve(&missing_static, Appearance::Light).name,
        "One Light"
    );
}

#[test]
fn problems_of_the_last_scan_are_kept() {
    let mut registry = ThemeRegistry::with_bundled();
    registry.replace_user_themes(UserThemes {
        families: Vec::new(),
        problems: vec![ThemeFileProblem {
            path: "bad.json".into(),
            message: "nope".into(),
        }],
    });
    assert_eq!(registry.problems().len(), 1);
    registry.replace_user_themes(UserThemes::default());
    assert!(registry.problems().is_empty());
}

fn family(text: &str) -> ThemeFamily {
    import_family(text).unwrap().family
}

fn ext(id: &str) -> ThemeSource {
    ThemeSource::Extension(ExtensionId::new(id))
}

#[test]
fn extension_families_are_listed_tagged_and_removed_by_source_only() {
    let mut registry = ThemeRegistry::with_bundled();
    registry.replace_user_themes(scan_of(&[("gruvbox.json", GRUVBOX)]));
    registry.insert_theme_families(&ext("acme.ayu"), [family(AYU)]);

    let ayu = registry
        .list()
        .into_iter()
        .find(|meta| meta.name == "Ayu Dark")
        .expect("Ayu Dark listed");
    assert_eq!(ayu.source, ext("acme.ayu"));
    assert!(registry.get("Ayu Light").is_some());
    assert_eq!(registry.names().len(), 2 + 3 + 6);

    assert_eq!(registry.remove_by_source(&ext("acme.ayu")), 3);
    assert!(registry.get("Ayu Dark").is_none());
    assert_eq!(registry.remove_by_source(&ext("acme.ayu")), 0, "idempotent");
    // User and bundled themes survive an extension unload.
    assert!(registry.get("Gruvbox Light").is_some());
    assert!(registry.get("One Dark").is_some());
}

#[test]
fn user_beats_extension_beats_bundled_and_later_insert_wins_within_a_source() {
    let one = |text: &str| {
        family(&format!(
            r##"{{ "themes": [{{ "name": "One Dark", "appearance": "dark", "style": {{ "text": "{text}" }} }}] }}"##
        ))
    };
    let text_of = |registry: &ThemeRegistry| registry.get("One Dark").unwrap().colors.text;
    let color = |hex: &str| crate::color::parse_color(hex).unwrap();

    let mut registry = ThemeRegistry::with_bundled();
    registry.insert_theme_families(&ext("a"), [one("#111111")]);
    registry.insert_theme_families(&ext("a"), [one("#222222")]);
    assert_eq!(text_of(&registry), color("#222222"), "later insert wins");
    assert_eq!(registry.list()[0].source, ext("a"));

    registry.insert_theme_families(&ThemeSource::User, [one("#333333")]);
    assert_eq!(text_of(&registry), color("#333333"), "user beats extension");
    assert_eq!(registry.names(), ["One Dark", "One Light"], "listed once");

    assert_eq!(registry.remove_user_themes(), 1);
    assert_eq!(text_of(&registry), color("#222222"));
    registry.remove_by_source(&ext("a"));
    assert_ne!(text_of(&registry), color("#222222"), "bundled again");
    assert_eq!(registry.list()[0].source, ThemeSource::Bundled);
}
