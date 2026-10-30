//! Extension-contributed themes through the GPUI globals: insert, select, unload, and the
//! change signal (E22-S04).

use gpui::{Subscription, TestAppContext};
use oxikube_settings::{SettingsStore, init_with_dir as init_settings};
use oxikube_theme::import::import_family;
use oxikube_theme::{
    ActiveTheme, ActiveThemeExt as _, Appearance, ExtensionId, SystemAppearance, ThemeSource,
    init_with_dir, insert_theme_families, remove_themes_by_source,
};
use std::cell::Cell;
use std::rc::Rc;

const AYU: &str = include_str!("fixtures/ayu.json");

fn setup(cx: &mut TestAppContext) -> (Rc<Cell<usize>>, Subscription) {
    let changes = Rc::new(Cell::new(0));
    let counter = changes.clone();
    let subscription = cx.update(|cx| {
        SystemAppearance::init(cx);
        SystemAppearance::set(cx, Appearance::Dark);
        init_with_dir(None, cx);
        cx.observe_global::<ActiveTheme>(move |_| counter.set(counter.get() + 1))
    });
    (changes, subscription)
}

fn source() -> ThemeSource {
    ThemeSource::Extension(ExtensionId::new("acme.ayu"))
}

#[gpui::test]
fn inserting_or_removing_unselected_themes_does_not_notify(cx: &mut TestAppContext) {
    let (changes, _subscription) = setup(cx);
    let family = import_family(AYU).expect("fixture imports").family;
    cx.update(|cx| insert_theme_families(cx, &source(), [family]));
    assert_eq!(changes.get(), 0);
    assert_eq!(cx.update(|cx| remove_themes_by_source(cx, &source())), 3);
    assert_eq!(changes.get(), 0);
}

#[gpui::test]
fn the_active_theme_is_readable_from_the_app_and_observed_exactly_once(cx: &mut TestAppContext) {
    let (changes, _subscription) = setup(cx);
    assert_eq!(cx.read(|cx| cx.active_theme().name.clone()), "One Dark");
    cx.update(|cx| {
        let registry = oxikube_theme::ThemeRegistry::global(cx);
        assert!(registry.get("Ayu Dark").is_none());
    });
    // A pinned mode change switches the default theme: one notification, a repeat is silent.
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    assert_eq!(changes.get(), 1);
    assert_eq!(cx.read(|cx| cx.active_theme().name.clone()), "One Light");
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    assert_eq!(changes.get(), 1);
}

#[gpui::test]
fn a_selected_extension_theme_activates_on_insert_and_falls_back_on_removal(
    cx: &mut TestAppContext,
) {
    let config = tempfile::tempdir().expect("temp dir");
    cx.update(|cx| init_settings(config.path(), cx));
    let (changes, _subscription) = setup(cx);
    cx.update(|cx| {
        gpui::UpdateGlobal::update_global(cx, |store: &mut SettingsStore, _| {
            store
                .set_user_settings(r#"{ "theme": "Ayu Dark" }"#)
                .expect("valid settings")
        });
    });
    // The selected theme is unknown for now, so the appearance default stays active.
    assert_eq!(cx.read(|cx| cx.active_theme().name.clone()), "One Dark");
    let before = changes.get();

    let family = import_family(AYU).expect("fixture imports").family;
    cx.update(|cx| insert_theme_families(cx, &source(), [family]));
    assert_eq!(cx.read(|cx| cx.active_theme().name.clone()), "Ayu Dark");
    assert_eq!(changes.get(), before + 1, "insert notifies once");

    cx.update(|cx| remove_themes_by_source(cx, &source()));
    assert_eq!(cx.read(|cx| cx.active_theme().name.clone()), "One Dark");
    assert_eq!(changes.get(), before + 2, "removal notifies once");
}

#[gpui::test]
fn dropping_the_subscription_releases_the_observer(cx: &mut TestAppContext) {
    let config = tempfile::tempdir().expect("temp dir");
    cx.update(|cx| init_settings(config.path(), cx));
    let (changes, subscription) = setup(cx);
    cx.update(|cx| {
        gpui::UpdateGlobal::update_global(cx, |store: &mut SettingsStore, _| {
            store
                .set_user_settings(r#"{ "theme": "Ayu Dark" }"#)
                .expect("valid settings")
        });
    });
    drop(subscription);
    // This insert changes the active theme, which would notify a live observer.
    let family = import_family(AYU).expect("fixture imports").family;
    cx.update(|cx| insert_theme_families(cx, &source(), [family]));
    assert_eq!(cx.read(|cx| cx.active_theme().name.clone()), "Ayu Dark");
    assert_eq!(changes.get(), 0, "a dropped observer never runs");
}
