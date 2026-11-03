//! Hot reload semantics under `#[gpui::test]` without the watcher thread: the directory is
//! rescanned with [`reload_now`], which is what the watcher's apply task does for each change.
//!
//! Covers add / replace / remove of theme files, keep-last-good for an invalid file, the toast
//! signal (once per file and message), and the active theme following its file on disk.

use gpui::{App, Subscription, TestAppContext};
use oxikube_settings::init_with_dir as init_settings;
use oxikube_theme::global::{ActiveTheme, init_with_dir, reload_now};
use oxikube_theme::user_dir::themes_dir;
use oxikube_theme::{
    Appearance, ExtensionId, SystemAppearance, ThemeNotice, ThemeRegistry, ThemeSource,
    insert_theme_families, pending_file_problems, subscribe_notices,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

const AYU: &str = include_str!("fixtures/ayu.json");
const GRUVBOX: &str = include_str!("fixtures/gruvbox.json");

struct Harness {
    _config: tempfile::TempDir,
    themes: PathBuf,
    changes: Rc<Cell<usize>>,
    _subscription: Subscription,
}

fn setup(cx: &mut TestAppContext, user_settings: &str) -> Harness {
    let config = tempfile::tempdir().expect("create temp config dir");
    std::fs::write(config.path().join("settings.json"), user_settings)
        .expect("write settings.json fixture");
    let themes = themes_dir(config.path());
    std::fs::create_dir_all(&themes).expect("create themes dir");
    let changes = Rc::new(Cell::new(0));
    let counter = changes.clone();
    let subscription = cx.update(|cx| {
        init_settings(config.path(), cx);
        SystemAppearance::init(cx);
        SystemAppearance::set(cx, Appearance::Dark);
        init_with_dir(Some(&themes), cx);
        cx.observe_global::<ActiveTheme>(move |_| counter.set(counter.get() + 1))
    });
    Harness {
        _config: config,
        themes,
        changes,
        _subscription: subscription,
    }
}

fn names(cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| ThemeRegistry::global(cx).names())
}

fn has(cx: &TestAppContext, name: &str) -> bool {
    names(cx).iter().any(|n| n == name)
}

fn reload(cx: &mut TestAppContext) {
    cx.update(reload_now);
}

fn collect(cx: &mut TestAppContext) -> (Rc<RefCell<Vec<ThemeNotice>>>, Subscription) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let subscription = cx.update(|cx: &mut App| {
        subscribe_notices(cx, move |n, _| sink.borrow_mut().push(n.clone()))
    });
    (seen, subscription)
}

#[gpui::test]
fn files_added_replaced_and_removed_update_the_registry(cx: &mut TestAppContext) {
    let harness = setup(cx, "{}");
    assert_eq!(names(cx).len(), 2);

    std::fs::write(harness.themes.join("ayu.json"), AYU).unwrap();
    reload(cx);
    assert!(has(cx, "Ayu Dark") && has(cx, "Ayu Mirage"));

    // Replace the file with a different family: the old family's themes go with it.
    std::fs::write(harness.themes.join("ayu.json"), GRUVBOX).unwrap();
    reload(cx);
    assert!(!has(cx, "Ayu Dark"));
    assert!(has(cx, "Gruvbox Dark"));

    std::fs::remove_file(harness.themes.join("ayu.json")).unwrap();
    reload(cx);
    assert_eq!(names(cx), ["One Dark", "One Light"]);
}

#[gpui::test]
fn an_invalid_file_keeps_the_old_version_and_raises_one_notice_with_the_path(
    cx: &mut TestAppContext,
) {
    let harness = setup(cx, "{}");
    let file = harness.themes.join("ayu.json");
    std::fs::write(&file, AYU).unwrap();
    reload(cx);
    let (seen, _subscription) = collect(cx);

    std::fs::write(&file, "{ \"name\": \"Ayu\", \"themes\": [").unwrap();
    reload(cx);
    assert!(has(cx, "Ayu Dark"), "the last good version stays installed");
    {
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        let ThemeNotice::BadThemeFile { path, .. } = &seen[0] else {
            panic!("expected a bad-file notice, got {:?}", seen[0]);
        };
        assert_eq!(path, &file);
        assert!(
            seen[0].message().contains("ayu.json"),
            "{}",
            seen[0].message()
        );
    }
    assert_eq!(cx.update(pending_file_problems).len(), 1);

    // The same breakage seen again says nothing more (a half-saved file does not spam).
    reload(cx);
    reload(cx);
    assert_eq!(seen.borrow().len(), 1);

    // Fixing the file clears it; breaking it again announces anew.
    std::fs::write(&file, AYU).unwrap();
    reload(cx);
    assert!(cx.update(pending_file_problems).is_empty());
    std::fs::write(&file, "nope").unwrap();
    reload(cx);
    assert_eq!(seen.borrow().len(), 2);
    assert!(has(cx, "Ayu Dark"));
}

#[gpui::test]
fn a_user_reload_does_not_remove_bundled_or_extension_themes(cx: &mut TestAppContext) {
    let harness = setup(cx, "{}");
    let source = ThemeSource::Extension(ExtensionId::new("acme.themes"));
    cx.update(|cx| {
        insert_theme_families(
            cx,
            &source,
            [oxikube_theme::import_family(GRUVBOX).unwrap().family],
        );
    });
    std::fs::write(harness.themes.join("ayu.json"), AYU).unwrap();
    reload(cx);
    std::fs::remove_file(harness.themes.join("ayu.json")).unwrap();
    std::fs::write(harness.themes.join("bad.json"), "{").unwrap();
    reload(cx);
    assert!(has(cx, "One Dark") && has(cx, "One Light"));
    assert!(has(cx, "Gruvbox Dark"), "the extension's themes survive");
    assert!(!has(cx, "Ayu Dark"));
}

#[gpui::test]
fn the_active_theme_follows_its_file_on_disk(cx: &mut TestAppContext) {
    let harness = setup(cx, r#"{ "theme": "Ayu Dark" }"#);
    let file = harness.themes.join("ayu.json");
    std::fs::write(&file, AYU).unwrap();
    reload(cx);
    let original = cx.read(ActiveTheme::get);
    assert_eq!(original.name, "Ayu Dark");
    let seen = harness.changes.get();

    // Edit one colour of the active theme in place: observers see one change.
    let edited = AYU.replacen("#0d1016ff", "#101820ff", 1);
    assert_ne!(edited, AYU);
    std::fs::write(&file, edited).unwrap();
    reload(cx);
    let updated = cx.read(ActiveTheme::get);
    assert_eq!(updated.name, "Ayu Dark");
    assert_ne!(*updated, *original, "the new colours are active");
    assert_eq!(harness.changes.get(), seen + 1);

    // Deleting the file falls back to the default of the theme's appearance.
    std::fs::remove_file(&file).unwrap();
    reload(cx);
    assert_eq!(cx.read(|cx| ActiveTheme::get(cx).name.clone()), "One Dark");
    // And the file coming back restores it.
    std::fs::write(&file, AYU).unwrap();
    reload(cx);
    assert_eq!(cx.read(|cx| ActiveTheme::get(cx).name.clone()), "Ayu Dark");
}
