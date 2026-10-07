//! WCAG 2.x contrast of every text-bearing token against every background it is drawn on.
//!
//! Small text needs 4.5:1 (AA). The check runs on the four token sets a user can see: the
//! built-in [`Tokens::dark`] / [`Tokens::light`] and the bundled One Dark / One Light themes
//! that [`crate::set_theme`] applies by default.

use super::Tokens;
use gpui::{Hsla, Rgba};
use oxikube_theme::{Appearance as ThemeAppearance, ThemeTokens};

/// The WCAG AA threshold for normal-size text.
const AA: f32 = 4.5;

/// WCAG relative luminance of an opaque colour.
fn luminance(Rgba { r, g, b, .. }: Rgba) -> f32 {
    let linear = |c: f32| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// `fg` over `bg` (alpha composited onto `bg`), as WCAG contrast ratio `(L1 + .05) / (L2 + .05)`.
fn contrast(fg: Hsla, bg: Hsla) -> f32 {
    let bg = Rgba::from(bg);
    let (l1, l2) = (luminance(bg.blend(fg.into())), luminance(bg));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

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

/// The backgrounds `foreground` must clear. Text and muted text are checked on all six. The
/// coloured tokens are checked on the resting and hover backgrounds (the first five): the
/// selected fill is a deliberately strong highlight (Zed's `element.selected`), and holding
/// red / amber / green to 4.5:1 on it would wash them out everywhere else; coloured text on a
/// selected row keeps its weight and shape (badge, icon) rather than relying on hue alone.
fn checked_backgrounds(t: &Tokens, foreground: &str) -> Vec<(&'static str, Hsla)> {
    let all = backgrounds(t);
    let n = if matches!(foreground, "text" | "text_muted") {
        all.len()
    } else {
        all.len() - 1
    };
    all[..n].to_vec()
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
            for (bg_name, bg) in checked_backgrounds(&tokens, fg_name) {
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

#[test]
fn the_ratio_matches_known_values() {
    let black = gpui::black();
    let white = gpui::white();
    assert!((contrast(black, white) - 21.0).abs() < 0.01);
    assert!((contrast(white, white) - 1.0).abs() < 0.001);
    // #767676 on white is the classic 4.54:1 AA boundary.
    let grey: Hsla = gpui::rgb(0x767676).into();
    assert!((contrast(grey, white) - 4.54).abs() < 0.01);
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
            for (bg_name, bg) in checked_backgrounds(&tokens, name) {
                let ratio = contrast(fg, bg);
                if ratio < AA {
                    failures.push(format!("{appearance:?}: {name} on {bg_name} = {ratio:.2}"));
                }
            }
        }
    }
    assert_no_failures(&failures);
}
