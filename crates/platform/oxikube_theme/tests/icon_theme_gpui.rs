//! Icon themes in a GPUI app (E22-S11): the `icon_theme` setting, the `icon_themes/` directory
//! and extension loads resolving into [`ActiveIconTheme`] and the per-thread paths drawing reads.
//!
//! Every test scans the directory once (`init_icon_themes_with_dir`, no watcher thread), so
//! GPUI's deterministic scheduler sees no foreign threads.

use gpui::{AssetSource as _, TestAppContext, UpdateGlobal as _};
use oxikube_assets::IconName;
use oxikube_settings::{Settings as _, SettingsStore, init_with_dir as init_settings};
use oxikube_theme::icons::{
    ActiveIconTheme, DEFAULT_ICON_THEME, IconThemeSettings, apply_user_icon_scan, icon_path,
    icon_themes_dir, init_icon_themes_with_dir, kind_icon_path, load_icon_theme,
    read_icon_theme_file, remove_icon_themes, scan_icon_themes_dir, status_icon_path,
};
use oxikube_theme::{Appearance, ExtensionId, SystemAppearance, ThemeRegistry, ThemeSource};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icon_themes")
}

/// Copies the fixture icon themes into `<config>/icon_themes/`, writes `settings.json` and
/// installs settings, themes and icon themes.
fn setup(cx: &mut TestAppContext, user: Option<&str>) -> tempfile::TempDir {
    let config = tempfile::tempdir().expect("create temp config dir");
    if let Some(user) = user {
        std::fs::write(config.path().join("settings.json"), user).expect("write settings.json");
    }
    let dir = icon_themes_dir(config.path());
    copy_dir(&fixtures(), &dir);
    cx.update(|cx| {
        init_settings(config.path(), cx);
        SystemAppearance::init(cx);
        SystemAppearance::set(cx, Appearance::Dark);
        oxikube_theme::init_with_dir(None, cx);
        init_icon_themes_with_dir(Some(&dir), cx);
    });
    config
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read fixtures") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy fixture");
        }
    }
}

fn set_user_settings(cx: &mut TestAppContext, text: &str) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(text).expect("test settings parse")
        })
    });
}

fn active_name(cx: &TestAppContext) -> String {
    cx.read(|cx| ActiveIconTheme::get(cx).theme().name.clone())
}

#[gpui::test]
fn default_json_selects_lucide(cx: &mut TestAppContext) {
    let _config = setup(cx, None);
    assert_eq!(active_name(cx), DEFAULT_ICON_THEME);
    cx.read(|cx| {
        assert_eq!(
            IconThemeSettings::get_global(cx),
            &IconThemeSettings::default()
        )
    });
    assert_eq!(kind_icon_path("", "Pod"), IconName::Box.path());
    assert_eq!(status_icon_path("Running"), IconName::CircleCheck.path());
}

#[gpui::test]
fn switching_the_icon_theme_changes_the_path_pod_resolves_to(cx: &mut TestAppContext) {
    let _config = setup(cx, None);
    let lucide_pod = kind_icon_path("", "Pod");
    assert_eq!(lucide_pod, IconName::Box.path());

    set_user_settings(cx, r#"{ "icon_theme": "Kube Test Icons" }"#);
    assert_eq!(active_name(cx), "Kube Test Icons");
    let themed_pod = kind_icon_path("", "Pod");
    assert_ne!(themed_pod, lucide_pod);
    assert!(themed_pod.ends_with("/kube/pod.svg"), "{themed_pod}");
    let svg = oxikube_assets::Assets
        .load(&themed_pod)
        .expect("asset source answers")
        .expect("the theme's file is served");
    assert!(svg.starts_with(b"<svg"));
    assert!(status_icon_path("Running").ends_with("/kube/running.svg"));
    // Not in the theme: Lucide.
    assert_eq!(kind_icon_path("", "Secret"), IconName::KeyRound.path());

    set_user_settings(cx, r#"{ "icon_theme": "Zed Sample Icons" }"#);
    assert_eq!(
        kind_icon_path("", "Pod"),
        lucide_pod,
        "a Zed theme has no kinds"
    );
    assert!(icon_path(IconName::ChevronRight).ends_with("chevron-right.svg"));

    set_user_settings(cx, "{}");
    assert_eq!(active_name(cx), DEFAULT_ICON_THEME);
    assert_eq!(
        icon_path(IconName::ChevronRight),
        IconName::ChevronRight.path()
    );
}

#[gpui::test]
fn a_mode_selection_follows_the_system_appearance(cx: &mut TestAppContext) {
    let _config = setup(
        cx,
        Some(
            r#"{ "icon_theme": { "mode": "system", "light": "Zed Sample Icons", "dark": "Kube Test Icons" } }"#,
        ),
    );
    assert_eq!(active_name(cx), "Kube Test Icons");
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    assert_eq!(active_name(cx), "Zed Sample Icons");
}

#[gpui::test]
fn an_unknown_name_shows_lucide(cx: &mut TestAppContext) {
    let _config = setup(cx, Some(r#"{ "icon_theme": "Not Installed" }"#));
    assert_eq!(active_name(cx), DEFAULT_ICON_THEME);
    assert_eq!(kind_icon_path("", "Pod"), IconName::Box.path());
}

#[gpui::test]
fn an_extension_icon_theme_loads_and_unloads_by_source(cx: &mut TestAppContext) {
    let _config = setup(cx, Some(r#"{ "icon_theme": "Extension Icons" }"#));
    assert_eq!(active_name(cx), DEFAULT_ICON_THEME, "not loaded yet");

    let root = fixtures();
    let mut read = read_icon_theme_file(&root.join("kube.json"), &root).expect("fixture reads");
    read.family.themes[0].name = "Extension Icons".into();
    let ext = ThemeSource::Extension(ExtensionId::new("kube-icons"));
    let names = cx.update(|cx| load_icon_theme(cx, &ext, read));
    assert_eq!(names, ["Extension Icons"]);
    assert_eq!(
        active_name(cx),
        "Extension Icons",
        "the selection takes effect"
    );
    assert!(kind_icon_path("", "Pod").ends_with("/kube/pod.svg"));

    let removed = cx.update(|cx| remove_icon_themes(cx, &ext));
    assert_eq!(removed, 1);
    assert_eq!(active_name(cx), DEFAULT_ICON_THEME);
    cx.read(|cx| {
        let listed: Vec<String> = ThemeRegistry::global(cx)
            .list_icon_themes()
            .into_iter()
            .map(|meta| meta.name)
            .collect();
        assert_eq!(
            listed,
            ["Kube Test Icons", DEFAULT_ICON_THEME, "Zed Sample Icons"]
        );
    });
}

#[gpui::test]
fn a_rescan_with_unchanged_files_keeps_the_active_theme_and_its_asset_paths(
    cx: &mut TestAppContext,
) {
    let config = setup(cx, Some(r#"{ "icon_theme": "Kube Test Icons" }"#));
    let dir = icon_themes_dir(config.path());
    let active = cx.read(ActiveIconTheme::get);
    let pod = kind_icon_path("", "Pod");
    assert!(pod.ends_with("/kube/pod.svg"), "{pod}");

    // A save elsewhere in `icon_themes/` rescans every file; unchanged ones keep their mount.
    std::fs::write(dir.join("unrelated.txt"), "x").expect("write unrelated file");
    cx.update(|cx| apply_user_icon_scan(cx, scan_icon_themes_dir(&dir)));
    assert!(
        Arc::ptr_eq(&active, &cx.read(ActiveIconTheme::get)),
        "an unchanged active theme is not re-resolved"
    );
    assert_eq!(
        kind_icon_path("", "Pod"),
        pod,
        "unchanged icons keep their asset path (GPUI's sprite atlas keys by path)"
    );

    // Editing the active theme's SVG remounts it under a new path, so no stale picture shows.
    std::fs::write(
        dir.join("kube/pod.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="4"/></svg>"#,
    )
    .expect("edit pod.svg");
    cx.update(|cx| apply_user_icon_scan(cx, scan_icon_themes_dir(&dir)));
    let edited = kind_icon_path("", "Pod");
    assert_ne!(edited, pod, "an edited file is served under a new path");
    assert!(edited.ends_with("/kube/pod.svg"), "{edited}");
}
