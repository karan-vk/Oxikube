//! GPUI-level behaviour of the theme globals: setting, system appearance and user themes
//! resolving into [`ActiveTheme`], observers firing once per real change.
//!
//! Every test uses `init_with_dir` (no watcher thread), so GPUI's deterministic scheduler sees
//! no foreign threads; the watcher is covered in `watch_gpui.rs` and `src/watcher.rs`.

use gpui::{App, Subscription, TestAppContext, UpdateGlobal as _};
use oxikube_settings::{SettingsStore, init_with_dir as init_settings};
use oxikube_theme::global::{ActiveTheme, apply_user_scan, init_with_dir};
use oxikube_theme::user_dir::{scan_dir, themes_dir};
use oxikube_theme::{
    Appearance, SystemAppearance, ThemeNotice, ThemeRegistry, pending_unknown_theme,
    subscribe_notices,
};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

const AYU: &str = include_str!("fixtures/ayu.json");
const DROPPED: &str = r##"{ "name": "Dropped", "themes": [
  { "name": "Dropped Dark", "appearance": "dark", "style": { "text": "#010203" } },
  { "name": "Dropped Light", "appearance": "light", "style": { "text": "#040506" } } ] }"##;

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
    let config = tempfile::tempdir().expect("create temp config dir");
    if let Some(user) = user {
        std::fs::write(config.path().join("settings.json"), user)
            .expect("write settings.json fixture");
    }
    let themes = themes_dir(config.path());
    std::fs::create_dir_all(&themes).expect("create themes dir");
    std::fs::write(themes.join("ayu.json"), AYU).expect("write ayu.json fixture");
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
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(text)
                .expect("test settings text parses")
        })
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
    let harness = setup(cx, Appearance::Dark, Some(r#"{ "theme": "Dropped Dark" }"#));
    assert_eq!(active_name(cx), "One Dark", "not installed yet");
    let dropped = DROPPED.to_owned();

    // The file arrives (the watcher does this in the app; tests feed the scan directly).
    std::fs::write(harness.themes().join("dropped.json"), dropped).unwrap();
    cx.update(|cx| apply_user_scan(cx, scan_dir(&harness.themes())));
    assert_eq!(active_name(cx), "Dropped Dark");
    cx.read(|cx| {
        assert!(ThemeRegistry::global(cx).get("Dropped Light").is_some());
    });

    // And leaves again.
    std::fs::remove_file(harness.themes().join("dropped.json")).unwrap();
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
        let bundled = ThemeRegistry::with_bundled().names();
        assert_eq!(ThemeRegistry::global(cx).names(), bundled);
        // Idempotent.
        init_with_dir(Some(Path::new("/nonexistent")), cx);
        assert_eq!(ThemeRegistry::global(cx).names().len(), bundled.len());
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

// gpui-pre 0.3.7 offers no public way to make the test window report a new appearance
// (`TestWindow::simulate_appearance_change` is reachable only inside gpui), so the OS flip is
// simulated at the point where `SystemAppearance::follow`'s observer lands: the global.

#[gpui::test]
fn an_os_appearance_flip_rethemes_within_the_same_tick(cx: &mut TestAppContext) {
    let _harness = setup(cx, Appearance::Dark, None);
    assert_eq!(active_name(cx), "One Dark");
    // No `run_until_parked`: the global's observer runs inside the same update.
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    assert_eq!(active_name(cx), "One Light");
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Dark));
    assert_eq!(active_name(cx), "One Dark");
}

#[gpui::test]
fn a_pinned_mode_ignores_the_os_flip_and_editing_the_mode_takes_effect(cx: &mut TestAppContext) {
    let _harness = setup(
        cx,
        Appearance::Light,
        Some(r#"{ "theme": { "mode": "dark" } }"#),
    );
    assert_eq!(active_name(cx), "One Dark");
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    assert_eq!(active_name(cx), "One Dark", "mode dark wins over the OS");

    // The hot-reload path: the settings file changed, the store re-reads it.
    set_user_settings(cx, r#"{ "theme": { "mode": "system" } }"#);
    assert_eq!(active_name(cx), "One Light", "following the OS again");
    set_user_settings(cx, r#"{ "theme": { "mode": "light" } }"#);
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Dark));
    assert_eq!(active_name(cx), "One Light");
}

#[gpui::test]
fn an_invalid_mode_is_a_diagnostic_naming_the_key_and_keeps_the_theme(cx: &mut TestAppContext) {
    let _harness = setup(
        cx,
        Appearance::Light,
        Some(r#"{ "theme": { "mode": "dark" } }"#),
    );
    assert_eq!(active_name(cx), "One Dark");
    set_user_settings(cx, r#"{ "theme": { "mode": "sometimes" } }"#);
    let messages: Vec<String> = cx.read(|cx| {
        cx.global::<SettingsStore>()
            .diagnostics()
            .iter()
            .map(ToString::to_string)
            .collect()
    });
    assert!(
        messages
            .iter()
            .any(|message| message.contains("theme.mode") && message.contains("sometimes")),
        "{messages:?}"
    );
    assert_eq!(active_name(cx), "One Dark", "the last good value stays");
}

fn collect_notices(cx: &mut TestAppContext) -> (Rc<RefCell<Vec<ThemeNotice>>>, Subscription) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let subscription = cx.update(|cx| {
        subscribe_notices(cx, move |notice, _| sink.borrow_mut().push(notice.clone()))
    });
    (seen, subscription)
}

#[gpui::test]
fn an_unknown_theme_name_raises_one_notice_per_name(cx: &mut TestAppContext) {
    let harness = setup(cx, Appearance::Dark, None);
    let (seen, _subscription) = collect_notices(cx);

    set_user_settings(cx, r#"{ "theme": "Nope" }"#);
    assert_eq!(active_name(cx), "One Dark", "falls back");
    assert_eq!(
        *seen.borrow(),
        [ThemeNotice::UnknownTheme {
            name: "Nope".into(),
            fallback: "One Dark".into()
        }]
    );
    assert!(
        seen.borrow()[0].message().contains("\"Nope\""),
        "{}",
        seen.borrow()[0].message()
    );

    // Unrelated resolutions (a rescan, a system flip, another edit) say nothing more.
    cx.update(|cx| apply_user_scan(cx, scan_dir(&harness.themes())));
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    set_user_settings(cx, r#"{ "theme": "Nope", "clusters": {} }"#);
    assert_eq!(seen.borrow().len(), 1);

    // Another unknown name is a new notice; a valid one clears, so the first can be reported again.
    set_user_settings(cx, r#"{ "theme": "Also Nope" }"#);
    assert_eq!(seen.borrow().len(), 2);
    set_user_settings(cx, r#"{ "theme": "Ayu Dark" }"#);
    assert_eq!(seen.borrow().len(), 2);
    set_user_settings(cx, r#"{ "theme": "Nope" }"#);
    assert_eq!(seen.borrow().len(), 3);
}

#[gpui::test]
fn a_name_that_is_installed_later_is_not_reported_before_the_first_scan(cx: &mut TestAppContext) {
    // With a settings file naming a user theme, `init_with_dir` scans before it resolves for good.
    let _harness = setup(cx, Appearance::Dark, Some(r#"{ "theme": "Ayu Dark" }"#));
    let (seen, _subscription) = collect_notices(cx);
    assert!(seen.borrow().is_empty());
    assert_eq!(cx.update(pending_unknown_theme), None);
}

#[gpui::test]
fn notices_raised_before_a_subscriber_are_readable_as_pending(cx: &mut TestAppContext) {
    let _harness = setup(cx, Appearance::Dark, Some(r#"{ "theme": "Missing" }"#));
    let pending = cx.update(pending_unknown_theme);
    assert!(matches!(
        pending,
        Some(ThemeNotice::UnknownTheme { ref name, .. }) if name == "Missing"
    ));
}
