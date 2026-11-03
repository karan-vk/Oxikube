use super::*;
use crate::appearance::ThemeMode;
use crate::icons::{DEFAULT_ICON_THEME, read_icon_theme_file};
use crate::registry::ExtensionId;
use gpui::AssetSource as _;
use std::path::Path;

fn read(file: &str) -> IconThemeSource {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icon_themes");
    read_icon_theme_file(&root.join(file), &root).unwrap()
}

fn family(name: &str) -> IconThemeFamily {
    let mut theme = IconTheme::lucide();
    theme.name = name.to_owned();
    IconThemeFamily {
        name: name.to_owned(),
        author: String::new(),
        themes: vec![theme],
    }
}

fn names(registry: &ThemeRegistry) -> Vec<(String, ThemeSource)> {
    registry
        .list_icon_themes()
        .into_iter()
        .map(|meta| (meta.name, meta.source))
        .collect()
}

#[test]
fn lucide_is_bundled_and_the_default() {
    let registry = ThemeRegistry::with_bundled();
    assert_eq!(
        names(&registry),
        [(DEFAULT_ICON_THEME.to_owned(), ThemeSource::Bundled)]
    );
    assert_eq!(registry.default_icon_theme().name, DEFAULT_ICON_THEME);
    let unknown = ThemeSelection::Static("Nope".into());
    assert_eq!(
        registry.resolve_icon_theme(&unknown, Appearance::Dark).name,
        DEFAULT_ICON_THEME,
        "an unknown name resolves to Lucide"
    );
    // A bare registry (no bundled set) still has a default.
    assert_eq!(
        ThemeRegistry::default().default_icon_theme().name,
        DEFAULT_ICON_THEME
    );
}

#[test]
fn sources_insert_and_remove_independently_with_user_first() {
    let mut registry = ThemeRegistry::with_bundled();
    let ext = ThemeSource::Extension(ExtensionId::new("material"));
    registry.insert_icon_theme_families(&ext, [family("Material"), family("Shared")]);
    registry.replace_user_icon_themes(vec![("mine.json".into(), family("Shared"))], Vec::new());
    assert_eq!(
        names(&registry),
        [
            (DEFAULT_ICON_THEME.to_owned(), ThemeSource::Bundled),
            ("Material".to_owned(), ext.clone()),
            ("Shared".to_owned(), ThemeSource::User),
        ]
    );

    assert_eq!(registry.remove_icon_themes(&ext), 2);
    assert!(registry.get_icon_theme("Material").is_none());
    assert!(
        registry.get_icon_theme("Shared").is_some(),
        "the user's stays"
    );
    assert!(registry.get_icon_theme(DEFAULT_ICON_THEME).is_some());
    assert_eq!(
        registry.remove_icon_themes(&ext),
        0,
        "removing twice is a no-op"
    );
}

#[test]
fn a_rescan_replaces_the_user_themes_and_reports_duplicates() {
    let mut registry = ThemeRegistry::with_bundled();
    registry.replace_user_icon_themes(
        vec![
            ("a.json".into(), family("Mine")),
            ("b.json".into(), family("Mine")),
        ],
        Vec::new(),
    );
    assert_eq!(registry.icon_theme_problems().len(), 1);
    assert!(registry.icon_theme_problems()[0].path.ends_with("b.json"));

    registry.replace_user_icon_themes(Vec::new(), Vec::new());
    assert!(
        registry.get_icon_theme("Mine").is_none(),
        "a deleted file disappears"
    );
    assert!(registry.icon_theme_problems().is_empty());
}

#[test]
fn a_rescan_keeps_the_arc_of_an_unchanged_theme_and_replaces_a_changed_one() {
    let mut registry = ThemeRegistry::with_bundled();
    let mounted = IconThemeFamily::mount(read("kube.json"));
    registry.replace_user_icon_themes(vec![("kube.json".into(), mounted.clone())], Vec::new());
    let name = mounted.themes[0].name.clone();
    let before = registry.get_icon_theme(&name).expect("user theme");

    // The same mounted family again (what `apply_user_icon_scan` hands back for an unchanged
    // file): the registry keeps the existing Arc, so the active theme is untouched.
    registry.replace_user_icon_themes(vec![("kube.json".into(), mounted)], Vec::new());
    let after = registry.get_icon_theme(&name).expect("user theme");
    assert!(
        Arc::ptr_eq(&before, &after),
        "an unchanged theme keeps its Arc"
    );

    // A fresh mount (a changed file) serves under new asset paths: a new Arc.
    let remounted = IconThemeFamily::mount(read("kube.json"));
    registry.replace_user_icon_themes(vec![("kube.json".into(), remounted)], Vec::new());
    let changed = registry.get_icon_theme(&name).expect("user theme");
    assert!(
        !Arc::ptr_eq(&after, &changed),
        "a remounted theme is replaced"
    );
}

#[test]
fn a_dynamic_selection_picks_by_appearance() {
    let mut registry = ThemeRegistry::with_bundled();
    registry.insert_icon_theme_families(&ThemeSource::User, [family("Day"), family("Night")]);
    let selection = ThemeSelection::Dynamic {
        mode: ThemeMode::System,
        light: "Day".into(),
        dark: "Night".into(),
    };
    assert_eq!(
        registry
            .resolve_icon_theme(&selection, Appearance::Light)
            .name,
        "Day"
    );
    assert_eq!(
        registry
            .resolve_icon_theme(&selection, Appearance::Dark)
            .name,
        "Night"
    );
}

#[test]
fn loaded_files_are_served_until_their_theme_is_removed() {
    let mut registry = ThemeRegistry::with_bundled();
    let ext = ThemeSource::Extension(ExtensionId::new("kube-icons"));
    assert_eq!(
        registry.load_icon_theme(&ext, read("kube.json")),
        ["Kube Test Icons"]
    );
    let theme = registry.get_icon_theme("Kube Test Icons").unwrap();
    let pod = theme.file_icons["kind:Pod"].path.clone();
    drop(theme);
    assert!(oxikube_assets::Assets.load(&pod).unwrap().is_some());

    registry.remove_icon_themes(&ext);
    assert!(
        oxikube_assets::Assets.load(&pod).unwrap().is_none(),
        "nothing holds the theme, so its files are unmounted"
    );
}
