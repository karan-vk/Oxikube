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

// ---- oxikube_theme -> ThemeConfig (E05-S08) -------------------------------------------------

mod theme_config_tests {
    use super::*;
    use crate::tokens::Appearance;
    use gpui::Hsla;
    use oxikube_theme::color::parse_color;
    use oxikube_theme::{ActiveTheme, SystemAppearance, ThemeTokens, import_family};

    /// Zed's Ayu and Gruvbox, copied from `assets/themes` for tests only (MIT; see
    /// `oxikube_theme/tests/fixtures/LICENSES.md`).
    const AYU: &str = include_str!("../../../../platform/oxikube_theme/tests/fixtures/ayu.json");
    const GRUVBOX: &str =
        include_str!("../../../../platform/oxikube_theme/tests/fixtures/gruvbox.json");

    fn theme(source: &str, name: &str) -> ThemeTokens {
        import_family(source)
            .unwrap()
            .family
            .themes
            .into_iter()
            .find(|theme| theme.name == name)
            .unwrap_or_else(|| panic!("no theme {name}"))
    }

    fn color(text: &str) -> Hsla {
        parse_color(text).unwrap()
    }

    #[test]
    fn ayu_maps_onto_a_theme_config() {
        let ayu = theme(AYU, "Ayu Dark");
        let config = theme_config(&ayu);
        assert_eq!(config.name, "Ayu Dark");
        assert!(config.mode.is_dark());

        // Core colours come from the same tokens views read: content area = editor background.
        let colors = &config.colors;
        assert_eq!(colors.background.as_deref(), Some("#0d1016ff"));
        assert_eq!(colors.foreground.as_deref(), Some("#bfbdb6ff"));
        assert_eq!(colors.primary.as_deref(), Some("#5ac1feff"));
        assert_eq!(colors.danger.as_deref(), Some("#ef7177ff"));
        assert_eq!(colors.border.as_deref(), Some("#3f4043ff"));
        assert_eq!(
            serde_json::to_value(&config.colors).unwrap()["base.red"],
            "#ef7177ff",
            "from the terminal ANSI row"
        );

        assert!(config.highlight.is_some(), "highlight section is passed on");
    }

    /// The `highlight` section is Zed `style` data. (Without gpui-component's `tree-sitter`
    /// feature the library keeps a stub of the section type that ignores the data, so the JSON
    /// handed to it is what is checked here.)
    #[test]
    fn highlight_section_is_zed_style_data() {
        let highlight = config::highlight(&theme(AYU, "Ayu Dark"));
        assert_eq!(highlight["editor.background"], "#0d1016ff");
        assert_eq!(highlight["editor.foreground"], "#bfbdb6ff");
        assert_eq!(
            highlight["editor.line_number"].as_str().map(str::len),
            Some(9)
        );
        assert_eq!(highlight["error"], "#ef7177ff");
        assert_eq!(highlight["syntax"]["keyword"]["color"], "#ff8f3fff");
        assert_eq!(highlight["syntax"]["comment"]["color"], "#5c6773ff");

        let light = config::highlight(&theme(GRUVBOX, "Gruvbox Light"));
        assert_eq!(light["editor.background"], "#fbf1c7ff");
        assert_eq!(light["syntax"]["keyword"]["color"], "#9d0006ff");
    }

    #[test]
    fn gruvbox_light_maps_onto_a_light_config() {
        let config = theme_config(&theme(GRUVBOX, "Gruvbox Light"));
        assert!(!config.mode.is_dark());
        assert_eq!(config.colors.background.as_deref(), Some("#fbf1c7ff"));
    }

    #[test]
    fn every_fixture_theme_yields_a_full_config() {
        for source in [AYU, GRUVBOX, oxikube_assets::BUNDLED_THEME_FAMILIES[0].json] {
            for theme in import_family(source).unwrap().family.themes {
                let config = theme_config(&theme);
                assert_eq!(config.name, theme.name, "config did not deserialize");
                assert!(config.colors.background.is_some(), "{}", theme.name);
                let colors = serde_json::to_value(&config.colors).unwrap();
                assert!(colors["base.cyan.light"].is_string(), "{}", theme.name);
                assert!(config.highlight.is_some(), "{}", theme.name);
            }
        }
    }

    #[test]
    fn weights_and_oblique_map_to_what_the_library_accepts() {
        let mut theme = theme(AYU, "Ayu Dark");
        theme.syntax.styles.insert(
            "emphasis".into(),
            oxikube_theme::tokens::SyntaxStyle {
                color: None,
                font_style: Some(oxikube_theme::tokens::FontStyle::Oblique),
                font_weight: Some(650.0),
            },
        );
        let emphasis = &config::highlight(&theme)["syntax"]["emphasis"];
        assert_eq!(
            emphasis["font_style"], "italic",
            "the library has no oblique"
        );
        assert_eq!(emphasis["font_weight"], 700, "650 rounds to a CSS weight");
        assert!(emphasis.get("color").is_none());
    }

    #[gpui::test]
    fn set_theme_applies_mode_colours_and_highlight(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            let light = theme(AYU, "Ayu Light");
            set_theme(cx, &light);
            let applied = cx.theme();
            assert!(!applied.is_dark());
            assert_eq!(cx.tokens().appearance, Appearance::Light);
            assert_eq!(applied.highlight_theme.name, "Ayu Light");
            assert_eq!(
                applied.light_theme.name, "Ayu Light",
                "registered with the library"
            );
            assert_eq!(applied.colors.background, color("#fcfcfcff"));
            assert_eq!(applied.colors.primary, color("#3b9ee5ff"));
            let keyword = applied
                .highlight_theme
                .style
                .syntax
                .style("keyword")
                .unwrap();
            assert_eq!(keyword.color, Some(color("#fa8d3eff")));
            // The library derives what the config leaves out from the config's colours.
            assert_eq!(applied.colors.accent, cx.tokens().colors.element_hover);

            let dark = theme(AYU, "Ayu Dark");
            set_theme(cx, &dark);
            assert!(cx.theme().is_dark());
            assert_eq!(cx.theme().highlight_theme.name, "Ayu Dark");
            assert_eq!(cx.theme().colors.background, color("#0d1016ff"));
        });
    }

    #[gpui::test]
    fn set_tokens_after_set_theme_drops_the_theme_config(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            set_theme(cx, &theme(AYU, "Ayu Dark"));
            assert_eq!(cx.theme().highlight_theme.name, "Ayu Dark");
            set_tokens(cx, Tokens::light());
            assert!(!cx.theme().is_dark());
            assert_eq!(
                cx.theme().colors.background,
                Tokens::light().colors.background
            );
            // Back to the library's own light highlight theme, not Ayu's.
            assert_ne!(cx.theme().highlight_theme.name, "Ayu Dark");
        });
    }

    #[gpui::test]
    fn a_theme_set_before_init_is_applied_by_init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let ayu = theme(AYU, "Ayu Light");
            set_theme(cx, &ayu);
            crate::init(cx);
            assert_eq!(cx.theme().highlight_theme.name, "Ayu Light");
            assert_eq!(cx.tokens(), Tokens::from(&ayu));
        });
    }

    #[gpui::test]
    fn follow_active_theme_tracks_the_system_appearance(cx: &mut TestAppContext) {
        let subscription = cx.update(|cx| {
            crate::init(cx);
            oxikube_theme::init_with_dir(None, cx);
            SystemAppearance::set(cx, oxikube_theme::Appearance::Dark);
            oxikube_theme::refresh_active(cx);
            follow_active_theme(cx)
        });
        cx.update(|cx| {
            assert_eq!(cx.theme().highlight_theme.name, "One Dark");
            assert!(cx.theme().is_dark());
            assert_eq!(
                cx.tokens().colors.accent,
                ActiveTheme::get(cx).colors.text_accent
            );
        });

        cx.update(|cx| SystemAppearance::set(cx, oxikube_theme::Appearance::Light));
        cx.update(|cx| {
            assert_eq!(cx.theme().highlight_theme.name, "One Light");
            assert!(!cx.theme().is_dark());
            assert_eq!(cx.tokens().appearance, Appearance::Light);
        });
        drop(subscription);
    }
}
