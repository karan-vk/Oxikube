//! GPUI-level behaviour of the theme globals: setting, system appearance and user themes
//! resolving into [`ActiveTheme`], observers firing once per real change.
//!
//! Every test uses `init_with_dir` (no watcher thread), so GPUI's deterministic scheduler sees
//! no foreign threads; the watcher is covered in `watch_gpui.rs` and `src/watcher.rs`.

use gpui::{App, Subscription, TestAppContext, UpdateGlobal as _};
use oxikube_settings::{SettingsStore, init_with_dir as init_settings};
use oxikube_theme::global::{ActiveTheme, apply_user_scan, init_with_dir};
use oxikube_theme::user_dir::{scan_dir, themes_dir};
use oxikube_theme::{Appearance, SystemAppearance, ThemeRegistry};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

const AYU: &str = include_str!("fixtures/ayu.json");
const GRUVBOX: &str = include_str!("fixtures/gruvbox.json");

struct Harness {
    config: tempfile::TempDir,
    changes: Rc<Cell<usize>>,
    _subscription: Subscription,
}

impl Harness {
    fn themes(&self) -> std::path::PathBuf {
        themes_dir(self.config.path())
    }
}

/// Settings (with `user` as settings.json) plus themes over the config dir's `themes/`.
fn setup(cx: &mut TestAppContext, system: Appearance, user: Option<&str>) -> Harness {
    let config = tempfile::tempdir().unwrap();
    if let Some(user) = user {
        std::fs::write(config.path().join("settings.json"), user).unwrap();
    }
    let themes = themes_dir(config.path());
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::write(themes.join("ayu.json"), AYU).unwrap();
    let changes = Rc::new(Cell::new(0));
    let counter = changes.clone();
    let subscription = cx.update(|cx| {
        init_settings(config.path(), cx);
        SystemAppearance::init(cx);
        SystemAppearance::set(cx, system);
        init_with_dir(Some(&themes), cx);
        cx.observe_global::<ActiveTheme>(move |_| counter.set(counter.get() + 1))
    });
    Harness {
        config,
        changes,
        _subscription: subscription,
    }
}

fn active_name(cx: &TestAppContext) -> String {
    cx.read(|cx| ActiveTheme::get(cx).name.clone())
}

fn set_user_settings(cx: &mut TestAppContext, text: &str) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| store.set_user_settings(text).unwrap())
    });
}

#[gpui::test]
fn defaults_follow_the_system_appearance_between_one_light_and_one_dark(cx: &mut TestAppContext) {
    let harness = setup(cx, Appearance::Dark, None);
    assert_eq!(active_name(cx), "One Dark");
    let seen = harness.changes.get();

    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    assert_eq!(active_name(cx), "One Light");
    assert_eq!(harness.changes.get(), seen + 1, "one change notification");

    cx.update(|cx| SystemAppearance::set(cx, Appearance::Dark));
    assert_eq!(active_name(cx), "One Dark");
}

#[gpui::test]
fn the_theme_setting_by_name_ignores_the_system(cx: &mut TestAppContext) {
    let _harness = setup(cx, Appearance::Light, Some(r#"{ "theme": "Ayu Dark" }"#));
    assert_eq!(active_name(cx), "Ayu Dark");
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Dark));
    assert_eq!(active_name(cx), "Ayu Dark");
}

#[gpui::test]
fn the_theme_setting_with_mode_light_dark(cx: &mut TestAppContext) {
    let user = r#"{ "theme": { "mode": "system", "light": "Ayu Light", "dark": "Ayu Mirage" } }"#;
    let harness = setup(cx, Appearance::Light, Some(user));
    assert_eq!(active_name(cx), "Ayu Light");
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Dark));
    assert_eq!(active_name(cx), "Ayu Mirage");

    // Pinning the mode wins over the system, and a partial object keeps the other names.
    set_user_settings(
        cx,
        r#"{ "theme": { "mode": "light", "light": "Ayu Light", "dark": "Ayu Mirage" } }"#,
    );
    assert_eq!(active_name(cx), "Ayu Light");
    set_user_settings(cx, r#"{ "theme": { "mode": "dark" } }"#);
    assert_eq!(
        active_name(cx),
        "One Dark",
        "light/dark names default again"
    );
    let seen = harness.changes.get();

    // An unrelated settings edit does not touch the active theme or notify.
    set_user_settings(cx, r#"{ "theme": { "mode": "dark" }, "clusters": {} }"#);
    assert_eq!(harness.changes.get(), seen);
}

#[gpui::test]
fn settings_edits_switch_themes_once(cx: &mut TestAppContext) {
    let harness = setup(cx, Appearance::Dark, None);
    let before = harness.changes.get();
    set_user_settings(cx, r#"{ "theme": "Ayu Light" }"#);
    assert_eq!(active_name(cx), "Ayu Light");
    assert_eq!(harness.changes.get(), before + 1);
    cx.read(|cx| assert_eq!(ActiveTheme::get(cx).appearance, Appearance::Light));
}

#[gpui::test]
fn an_unknown_theme_name_falls_back_until_it_is_dropped_into_the_directory(
    cx: &mut TestAppContext,
) {
    let harness = setup(
        cx,
        Appearance::Dark,
        Some(r#"{ "theme": "Gruvbox Dark Hard" }"#),
    );
    assert_eq!(active_name(cx), "One Dark", "not installed yet");

    // The file arrives (the watcher does this in the app; tests feed the scan directly).
    std::fs::write(harness.themes().join("gruvbox.json"), GRUVBOX).unwrap();
    cx.update(|cx| apply_user_scan(cx, scan_dir(&harness.themes())));
    assert_eq!(active_name(cx), "Gruvbox Dark Hard");
    cx.read(|cx| {
        assert!(ThemeRegistry::global(cx).get("Gruvbox Light").is_some());
    });

    // And leaves again.
    std::fs::remove_file(harness.themes().join("gruvbox.json")).unwrap();
    cx.update(|cx| apply_user_scan(cx, scan_dir(&harness.themes())));
    assert_eq!(active_name(cx), "One Dark");
}

#[gpui::test]
fn rescans_that_change_nothing_do_not_notify(cx: &mut TestAppContext) {
    let harness = setup(cx, Appearance::Dark, None);
    let seen = harness.changes.get();
    cx.update(|cx| apply_user_scan(cx, scan_dir(&harness.themes())));
    cx.update(|cx| apply_user_scan(cx, scan_dir(&harness.themes())));
    assert_eq!(harness.changes.get(), seen);
}

#[gpui::test]
fn editing_the_active_user_theme_file_updates_it(cx: &mut TestAppContext) {
    let harness = setup(cx, Appearance::Dark, Some(r#"{ "theme": "Ayu Dark" }"#));
    let before = cx.read(|cx| ActiveTheme::get(cx).colors.text);
    let edited = AYU.replace("#bfbdb6ff", "#010203ff");
    std::fs::write(harness.themes().join("ayu.json"), edited).unwrap();
    cx.update(|cx| apply_user_scan(cx, scan_dir(&harness.themes())));
    let after = cx.read(|cx| ActiveTheme::get(cx).colors.text);
    assert_ne!(before, after);
}

#[gpui::test]
fn works_without_a_settings_store_or_a_themes_dir(cx: &mut TestAppContext) {
    cx.update(|cx: &mut App| {
        init_with_dir(None, cx);
        SystemAppearance::set(cx, Appearance::Light);
        assert_eq!(ActiveTheme::get(cx).name, "One Light");
        assert_eq!(ThemeRegistry::global(cx).names(), ["One Dark", "One Light"]);
        // Idempotent.
        init_with_dir(Some(Path::new("/nonexistent")), cx);
        assert_eq!(ThemeRegistry::global(cx).names().len(), 2);
    });
}

#[gpui::test]
fn active_theme_before_init_is_the_bundled_fallback(cx: &mut TestAppContext) {
    cx.read(|cx| {
        let theme = ActiveTheme::get(cx);
        assert_eq!(theme.name, "One Dark");
    });
}

#[gpui::test]
fn following_a_window_adopts_its_appearance_and_rethemes(cx: &mut TestAppContext) {
    let _harness = setup(cx, Appearance::Dark, None);
    let window = cx.add_window(|_, _| gpui::Empty);
    let window_appearance = window
        .update(cx, |_, window, _| Appearance::from(window.appearance()))
        .unwrap();
    // Pin the app to the opposite of whatever the test window reports, then follow it.
    cx.update(|cx| {
        SystemAppearance::set(
            cx,
            if window_appearance.is_dark() {
                Appearance::Light
            } else {
                Appearance::Dark
            },
        )
    });
    let subscription = window
        .update(cx, |_, window, cx| SystemAppearance::follow(window, cx))
        .unwrap();
    cx.read(|cx| assert_eq!(SystemAppearance::get(cx), window_appearance));
    assert_eq!(
        active_name(cx),
        if window_appearance.is_dark() {
            "One Dark"
        } else {
            "One Light"
        }
    );
    drop(subscription);
}
