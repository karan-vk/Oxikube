use super::*;
use crate::appearance::ThemeMode;

fn dynamic(mode: Option<ThemeMode>) -> ThemeSelectionContent {
    ThemeSelectionContent::Dynamic {
        mode,
        light: None,
        dark: None,
    }
}

#[test]
fn a_static_setting_is_replaced() {
    let mut content = ThemeSelectionContent::Static("One Dark".into());
    set_theme_name(&mut content, "Ayu Light", Appearance::Dark);
    assert_eq!(content, ThemeSelectionContent::Static("Ayu Light".into()));
}

#[test]
fn system_mode_edits_the_slot_of_the_system_appearance() {
    let mut content = dynamic(None);
    set_theme_name(&mut content, "X", Appearance::Light);
    assert_eq!(
        content,
        ThemeSelectionContent::Dynamic {
            mode: None,
            light: Some("X".into()),
            dark: None
        }
    );
    set_theme_name(&mut content, "Y", Appearance::Dark);
    assert_eq!(
        content,
        ThemeSelectionContent::Dynamic {
            mode: None,
            light: Some("X".into()),
            dark: Some("Y".into())
        }
    );
}

#[test]
fn a_fixed_mode_ignores_the_system_appearance() {
    let mut content = dynamic(Some(ThemeMode::Dark));
    set_theme_name(&mut content, "X", Appearance::Light);
    assert_eq!(
        content,
        ThemeSelectionContent::Dynamic {
            mode: Some(ThemeMode::Dark),
            light: None,
            dark: Some("X".into())
        }
    );
}

#[test]
fn the_slot_follows_the_selection() {
    let fixed = ThemeSelection::Static("a".into());
    assert_eq!(edited_slot(&fixed, Appearance::Dark), ThemeSlot::Any);
    let system = ThemeSelection::default();
    assert_eq!(edited_slot(&system, Appearance::Dark), ThemeSlot::Dark);
    assert_eq!(edited_slot(&system, Appearance::Light), ThemeSlot::Light);
    assert_eq!(ThemeSlot::Light.label(), "light theme");
}
