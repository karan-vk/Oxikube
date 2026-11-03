//! The list, the preview, and the ways out.

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_theme::Appearance;

use super::{Fixture, SYSTEM_SETTINGS};
use crate::theme_picker::{Row, delegate::ThemeDelegate};

/// `(group label, theme names)` in display order.
fn listing(f: &mut Fixture) -> Vec<(Appearance, Vec<String>)> {
    let picker = f.picker().expect("the picker is open");
    f.vcx.update(|_, cx| {
        let delegate: &ThemeDelegate = &picker.read(cx).delegate;
        let mut groups: Vec<(Appearance, Vec<String>)> = Vec::new();
        for row in delegate.rows() {
            match row {
                Row::Header { appearance, .. } => groups.push((*appearance, Vec::new())),
                Row::Theme { theme, .. } => groups
                    .last_mut()
                    .expect("a header comes first")
                    .1
                    .push(delegate.themes()[*theme].name.clone()),
            }
        }
        groups
    })
}

#[gpui::test]
fn it_lists_the_registry_grouped_with_the_current_theme_selected(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    assert_eq!(
        listing(&mut f),
        vec![
            (
                Appearance::Dark,
                vec!["Ayu Dark".into(), "Ayu Mirage".into(), "One Dark".into()]
            ),
            (
                Appearance::Light,
                vec!["Ayu Light".into(), "One Light".into()]
            ),
        ],
        "the appearance in effect is listed first"
    );
    assert_eq!(f.selected_name().as_deref(), Some("One Dark"));
    assert_eq!(f.active(), "One Dark");
}

#[gpui::test]
fn the_group_of_the_light_slot_comes_first_in_a_light_system(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Light, SYSTEM_SETTINGS);
    f.open();
    let groups = listing(&mut f);
    assert_eq!(groups[0].0, Appearance::Light);
    assert_eq!(f.selected_name().as_deref(), Some("One Light"));
    let caption = f.vcx.update(|_, cx| {
        crate::theme_picker::render::slot_caption(oxikube_theme::current_choice(cx))
    });
    assert!(
        caption.starts_with("Editing your light theme; mode: system"),
        "{caption}"
    );
}

#[gpui::test]
fn arrow_keys_preview_each_theme_and_write_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.keys("up");
    assert_eq!(f.selected_name().as_deref(), Some("Ayu Mirage"));
    assert_eq!(f.active(), "Ayu Mirage", "previewed as the selection moved");
    f.keys("up");
    assert_eq!(f.active(), "Ayu Dark");
    f.keys("down down");
    assert_eq!(f.active(), "One Dark");
    // Headers are skipped: down from the last dark theme lands on the first light one.
    f.keys("down");
    assert_eq!(f.selected_name().as_deref(), Some("Ayu Light"));
    assert_eq!(f.active(), "Ayu Light");
    assert_eq!(f.settings_text(), SYSTEM_SETTINGS, "previews are not saved");
}

#[gpui::test]
fn escape_reverts_the_preview_and_closes(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.keys("up up");
    assert_eq!(f.active(), "Ayu Dark");
    f.keys("escape");
    assert!(f.picker().is_none(), "closed");
    assert_eq!(f.active(), "One Dark", "back to the setting's theme");
    assert!(!f.vcx.update(|_, cx| oxikube_theme::is_previewing(cx)));
    assert_eq!(f.settings_text(), SYSTEM_SETTINGS);
}

#[gpui::test]
fn a_click_outside_counts_as_cancel_and_reverts(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.keys("up");
    assert_eq!(f.active(), "Ayu Mirage");
    f.vcx.simulate_click(
        gpui::point(gpui::px(2.), gpui::px(2.)),
        gpui::Modifiers::default(),
    );
    f.settle();
    assert!(f.picker().is_none(), "the click outside closed it");
    assert_eq!(f.active(), "One Dark");
    assert!(!f.vcx.update(|_, cx| oxikube_theme::is_previewing(cx)));
}

#[gpui::test]
fn toggling_the_host_again_closes_and_reverts(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.keys("up");
    f.open();
    assert!(f.picker().is_none());
    assert_eq!(f.active(), "One Dark");
}

#[gpui::test]
fn typing_filters_and_previews_the_best_match(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.type_text("mirage");
    assert_eq!(f.selected_name().as_deref(), Some("Ayu Mirage"));
    assert_eq!(f.active(), "Ayu Mirage");
    let groups = listing(&mut f);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].1, vec!["Ayu Mirage".to_owned()]);
    f.keys("escape");
    assert_eq!(f.active(), "One Dark");
}

#[gpui::test]
fn a_settings_reload_while_open_does_not_clobber_the_preview(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.keys("up");
    f.vcx.update(|_, cx| {
        oxikube_settings::SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(&SYSTEM_SETTINGS.replace("\"zoom\": 1", "\"zoom\": 2"))
                .expect("settings parse");
        });
    });
    f.settle();
    assert_eq!(f.active(), "Ayu Mirage");
    f.keys("escape");
    assert_eq!(f.active(), "One Dark");
}
