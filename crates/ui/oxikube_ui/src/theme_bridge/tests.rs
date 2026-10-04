use super::*;
use crate::{ActiveTokens, UiScale};
use gpui::{TestAppContext, px};
use gpui_component::ActiveTheme as _;

#[test]
fn apply_tokens_maps_colours_radii_and_font_size() {
    let tokens = Tokens::dark();
    let mut theme = Theme::default();
    apply_tokens(&mut theme, &tokens, UiScale::IDENTITY);

    assert_eq!(theme.colors.background, tokens.colors.background);
    assert_eq!(theme.colors.foreground, tokens.colors.text);
    assert_eq!(theme.colors.primary, tokens.colors.accent);
    assert_eq!(theme.colors.table_head, tokens.colors.surface);
    assert_eq!(theme.colors.table_row_border, tokens.colors.border_variant);
    assert_eq!(theme.colors.danger, tokens.colors.error);
    assert_eq!(theme.radius, px(6.));
    assert_eq!(theme.radius_lg, px(10.));
    assert_eq!(theme.font_size, px(14.));
}

#[test]
fn apply_tokens_scales_sizes_with_the_zoom() {
    let mut theme = Theme::default();
    apply_tokens(&mut theme, &Tokens::dark(), UiScale::new(2.0));
    assert_eq!(theme.font_size, px(28.));
    assert_eq!(theme.radius, px(12.));
}

#[test]
fn shade_stays_in_range() {
    let white = hsla(0., 0., 1., 1.);
    assert_eq!(shade(white, 0.5).l, 1.0);
    assert_eq!(shade(hsla(0., 0., 0., 1.), -0.5).l, 0.0);
}

#[gpui::test]
fn set_tokens_re_themes_the_component_library(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
        let light = Tokens::light();
        set_tokens(cx, light);
        assert_eq!(cx.tokens(), light);
        assert!(!cx.theme().is_dark());
        assert_eq!(cx.theme().colors.background, light.colors.background);

        let dark = Tokens::dark();
        set_tokens(cx, dark);
        assert!(cx.theme().is_dark());
        assert_eq!(cx.theme().colors.background, dark.colors.background);
        assert_eq!(cx.theme().colors.table_active, dark.colors.element_selected);
    });
}

#[gpui::test]
fn tokens_set_before_init_are_applied_by_init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let light = Tokens::light();
        set_tokens(cx, light);
        assert_eq!(cx.tokens(), light);
        crate::init(cx);
        assert_eq!(cx.tokens(), light);
        assert!(!cx.theme().is_dark());
        assert_eq!(cx.theme().colors.background, light.colors.background);
    });
}

#[gpui::test]
fn init_is_idempotent(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
        let dark = Tokens::dark();
        set_tokens(cx, dark);
        crate::init(cx);
        assert_eq!(cx.tokens(), dark);
    });
}
