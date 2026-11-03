//! What Enter writes.

use gpui::TestAppContext;
use oxikube_theme::Appearance;

use super::{Fixture, SYSTEM_SETTINGS};

#[gpui::test]
fn enter_writes_the_dark_slot_in_a_dark_system_keeping_comments(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.keys("up");
    f.keys("enter");
    assert!(f.picker().is_none(), "closed");
    let text = f.settings_text();
    assert!(text.contains("// keep me"), "{text}");
    assert!(text.contains("\"zoom\": 1"), "{text}");
    assert!(text.contains("\"dark\": \"Ayu Mirage\""), "{text}");
    assert!(
        text.contains("\"light\": \"One Light\""),
        "the other slot is untouched: {text}"
    );
    assert_eq!(f.active(), "Ayu Mirage", "the committed theme stays");
    assert!(
        !f.vcx.update(|_, cx| oxikube_theme::is_previewing(cx)),
        "the preview ended"
    );
}

#[gpui::test]
fn enter_writes_the_light_slot_in_a_light_system(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Light, SYSTEM_SETTINGS);
    f.open();
    f.keys("up");
    f.keys("enter");
    let text = f.settings_text();
    assert!(text.contains("\"light\": \"Ayu Light\""), "{text}");
    assert!(text.contains("\"dark\": \"One Dark\""), "{text}");
    assert_eq!(f.active(), "Ayu Light");
}

#[gpui::test]
fn a_static_setting_is_replaced_by_the_name(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, "{ \"theme\": \"One Dark\" }\n");
    f.open();
    f.keys("up up");
    f.keys("enter");
    assert!(
        f.settings_text().contains("\"theme\": \"Ayu Dark\""),
        "{}",
        f.settings_text()
    );
    assert_eq!(f.active(), "Ayu Dark");
}

#[gpui::test]
fn enter_on_the_current_theme_keeps_it(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.keys("enter");
    assert!(f.picker().is_none());
    assert_eq!(f.active(), "One Dark");
}

#[gpui::test]
fn enter_with_no_match_closes_without_writing(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, Appearance::Dark, SYSTEM_SETTINGS);
    f.open();
    f.type_text("zzzzzz");
    f.keys("enter");
    assert_eq!(f.settings_text(), SYSTEM_SETTINGS);
    assert_eq!(f.active(), "One Dark");
}
