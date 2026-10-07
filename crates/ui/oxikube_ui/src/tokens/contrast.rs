//! WCAG 2.x contrast of every text-bearing token against every background it is drawn on.
//!
//! Small text needs 4.5:1 (AA). The check runs on the four token sets a user can see: the
//! built-in [`Tokens::dark`] / [`Tokens::light`] and the bundled One Dark / One Light themes
//! that [`crate::set_theme`] applies by default. Every text token is held to 4.5:1 on every
//! background, the selected-row fill included: a selected table row still draws its status text.

use super::Tokens;
use gpui::Hsla;
use oxikube_theme::tokens::contrast_ratio as contrast;
use oxikube_theme::{Appearance as ThemeAppearance, ThemeTokens};

/// The WCAG AA threshold for normal-size text.
const AA: f32 = 4.5;

/// The four token sets, named.
fn sets() -> [(&'static str, Tokens); 4] {
    [
        ("Tokens::dark", Tokens::dark()),
        ("Tokens::light", Tokens::light()),
        (
            "One Dark",
            Tokens::from(ThemeTokens::fallback(ThemeAppearance::Dark)),
        ),
        (
            "One Light",
            Tokens::from(ThemeTokens::fallback(ThemeAppearance::Light)),
        ),
    ]
}

/// Every background text is drawn on: the content area, panels, popovers and the resting,
/// hovered and selected states of an element (table rows, buttons, tabs).
fn backgrounds(t: &Tokens) -> [(&'static str, Hsla); 6] {
    let c = &t.colors;
    [
        ("background", c.background),
        ("surface", c.surface),
        ("elevated_surface", c.elevated_surface),
        ("element", c.element),
        ("element_hover", c.element_hover),
        ("element_selected", c.element_selected),
    ]
}

/// Every text-bearing colour (the log level colours are `error`, `warning`, `info`, `text_muted`
/// and `text`).
fn foregrounds(t: &Tokens) -> [(&'static str, Hsla); 7] {
    let c = &t.colors;
    [
        ("text", c.text),
        ("text_muted", c.text_muted),
        ("success", c.success),
        ("warning", c.warning),
        ("info", c.info),
        ("error", c.error),
        ("accent", c.accent),
    ]
}

fn assert_no_failures(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "below {AA}:1:\n{}",
        failures.join("\n")
    );
}

#[test]
fn text_tokens_meet_wcag_aa_on_every_background() {
    let mut failures = Vec::new();
    for (set, tokens) in sets() {
        for (fg_name, fg) in foregrounds(&tokens) {
            for (bg_name, bg) in backgrounds(&tokens) {
                let ratio = contrast(fg, bg);
                if ratio < AA {
                    failures.push(format!("{set}: {fg_name} on {bg_name} = {ratio:.2}"));
                }
            }
        }
    }
    assert_no_failures(&failures);
}

#[test]
fn on_accent_is_readable_on_the_accent() {
    for (set, tokens) in sets() {
        let c = tokens.colors;
        let ratio = contrast(c.on_accent, c.accent);
        assert!(ratio >= AA, "{set}: on_accent on accent = {ratio:.2}");
    }
}

/// The `oxikube` status colours the tables and sidebar paint with.
#[test]
fn oxikube_status_colours_meet_wcag_aa() {
    let mut failures = Vec::new();
    for appearance in [ThemeAppearance::Dark, ThemeAppearance::Light] {
        let theme = ThemeTokens::fallback(appearance);
        let tokens = Tokens::from(theme);
        let o = &theme.oxikube;
        let fills = [
            ("status_running", o.status_running),
            ("status_pending", o.status_pending),
            ("status_failed", o.status_failed),
            ("status_succeeded", o.status_succeeded),
            ("status_unknown", o.status_unknown),
        ];
        for (name, fg) in fills {
            for (bg_name, bg) in backgrounds(&tokens) {
                let ratio = contrast(fg, bg);
                if ratio < AA {
                    failures.push(format!("{appearance:?}: {name} on {bg_name} = {ratio:.2}"));
                }
            }
        }
    }
    assert_no_failures(&failures);
}

/// The pod-name colours of the multi-pod log gutter, on the log row fills (plain, matched and
/// current match).
#[test]
fn log_source_colours_meet_wcag_aa() {
    let mut failures = Vec::new();
    for appearance in [ThemeAppearance::Dark, ThemeAppearance::Light] {
        let theme = ThemeTokens::fallback(appearance);
        let c = Tokens::from(theme).colors;
        let rows = [
            ("background", c.background),
            ("element", c.element),
            ("element_selected", c.element_selected),
        ];
        for (slot, fg) in theme.oxikube.log_sources.into_iter().enumerate() {
            for (bg_name, bg) in rows {
                let ratio = contrast(fg, bg);
                if ratio < AA {
                    failures.push(format!(
                        "{appearance:?}: log_sources[{slot}] on {bg_name} = {ratio:.2}"
                    ));
                }
            }
        }
    }
    assert_no_failures(&failures);
}
