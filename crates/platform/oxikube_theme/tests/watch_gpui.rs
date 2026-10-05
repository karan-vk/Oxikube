//! Hot reload end to end under `#[gpui::test]`: a theme file dropped into a temp `themes/`
//! directory appears in the registry's `list` through the real `notify` watcher thread, and
//! selecting it takes effect; deleting it removes it again.
//!
//! The watcher is an OS thread, which GPUI's deterministic scheduler forbids unless the test
//! allows parking; that is why this is the only test that enables it.

use gpui::TestAppContext;
use oxikube_settings::{SettingsStore, init_with_dir as init_settings};
use oxikube_theme::{ActiveTheme, Appearance, SystemAppearance, ThemeRegistry, init_watching_dir};
use std::time::{Duration, Instant};

const AYU: &str = include_str!("fixtures/ayu.json");

/// Runs the foreground executor and re-checks `done` until it holds or ten seconds pass. Real
/// time passes here: the file events come from the OS.
fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn names(cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| ThemeRegistry::global(cx).names())
}

#[gpui::test]
fn a_theme_file_dropped_into_the_directory_appears_and_can_be_selected(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let config = tempfile::tempdir().unwrap();
    let themes = config.path().join("themes");
    cx.update(|cx| {
        init_settings(config.path(), cx);
        SystemAppearance::set(cx, Appearance::Dark);
        init_watching_dir(&themes, cx);
    });
    assert_eq!(names(cx), ["One Dark", "One Light"]);

    // Drop the file in. The OS watch starts asynchronously (FSEvents), so rewrite it every
    // half second until it is seen rather than guessing a delay.
    let file = themes.join("ayu.json");
    let mut written: Option<Instant> = None;
    wait_until(cx, "Ayu Dark to be listed", |cx| {
        if written.is_none_or(|at| at.elapsed() > Duration::from_millis(500)) {
            std::fs::write(&file, AYU).unwrap();
            written = Some(Instant::now());
        }
        names(cx).iter().any(|name| name == "Ayu Dark")
    });
    assert_eq!(names(cx).len(), 5);

    // Selecting it takes effect.
    cx.update(|cx| {
        gpui::UpdateGlobal::update_global(cx, |store: &mut SettingsStore, _| {
            store
                .set_user_settings(r#"{ "theme": "Ayu Dark" }"#)
                .unwrap()
        });
    });
    assert_eq!(cx.read(|cx| ActiveTheme::get(cx).name.clone()), "Ayu Dark");

    // Deleting the file removes the themes; the selection falls back.
    std::fs::remove_file(&file).unwrap();
    wait_until(cx, "Ayu Dark to disappear", |cx| {
        !names(cx).iter().any(|name| name == "Ayu Dark")
    });
    assert_eq!(cx.read(|cx| ActiveTheme::get(cx).name.clone()), "One Dark");
}
