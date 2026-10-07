//! The level chips (JSON mode): one per level plus `text`, each showing or hiding its lines.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, px,
};
use oxikube_domain::log::LevelChip;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Selectable as _, h_flex};
use oxikube_ui::{ActiveTokens as _, Colors, Sizable as _, u};

use crate::view::LogView;

impl LogView {
    /// The level chips (JSON mode): one per level plus `text`, each showing or hiding its lines.
    pub(crate) fn level_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.options.json || !self.shows_json_controls() {
            return None;
        }
        let tokens = cx.tokens();
        let chips = LevelChip::ALL.map(|chip| {
            let id = format!("log-level-{}", chip.label());
            let selector = id.clone();
            let colour = chip_colour(chip, &tokens.colors);
            h_flex()
                .gap(u(tokens.spacing.xs))
                .items_center()
                .debug_selector(move || selector)
                .child(
                    div()
                        .size(u(px(6.)))
                        .rounded_full()
                        .bg(if self.levels.shows(chip) {
                            colour
                        } else {
                            tokens.colors.text_disabled
                        }),
                )
                .child(
                    Button::new(gpui::SharedString::from(id))
                        .label(chip.label())
                        .ghost()
                        .xsmall()
                        .selected(self.levels.shows(chip))
                        .on_click(cx.listener(move |view, _, _, cx| view.request_level(chip, cx))),
                )
        });
        Some(
            h_flex()
                .id("log-levels")
                .debug_selector(|| "log-levels".into())
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .items_center()
                .border_b_1()
                .border_color(tokens.colors.border_variant)
                .child(
                    div()
                        .text_color(tokens.colors.text_muted)
                        .text_size(u(tokens.font.small))
                        .child("Levels"),
                )
                .children(chips)
                .into_any_element(),
        )
    }
}

/// The dot of a chip: the level's accent (the `text` chip is neutral).
fn chip_colour(chip: LevelChip, colors: &Colors) -> gpui::Hsla {
    match chip {
        LevelChip::Trace | LevelChip::Debug => colors.text_muted,
        LevelChip::Info => colors.info,
        LevelChip::Warn => colors.warning,
        LevelChip::Error | LevelChip::Fatal => colors.error,
        LevelChip::Text => colors.text,
    }
}
