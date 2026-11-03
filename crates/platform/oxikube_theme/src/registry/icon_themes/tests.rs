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
