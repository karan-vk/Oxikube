//! The palette's rows and footer: category chip, title with the matched characters highlighted,
//! the key binding (or, for an unavailable command, why it cannot run), and the "Show all" toggle.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use oxikube_keymap::bindings_for_action_name;
use oxikube_ui::kbd::keycap;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::TOGGLE_SHOW_ALL_ACTION;
use super::delegate::CommandPaletteDelegate;
use crate::picker::fuzzy::highlighted_text;
use crate::picker::{Picker, match_label};

impl CommandPaletteDelegate {
    pub(super) fn render_row(
        &self,
        ix: usize,
        selected: bool,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Stateful<gpui::Div>> {
        let (found, row) = self.row(ix)?;
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let info = &row.info;
        let category = info.category().label();
        let (chip_hits, title_hits): (Vec<usize>, Vec<usize>) = {
            let mut chip = Vec::new();
            let mut title = Vec::new();
            for &position in &found.positions {
                if position < row.title_offset.saturating_sub(1) {
                    chip.push(position);
                } else if position >= row.title_offset {
                    title.push(position - row.title_offset);
                }
            }
            (chip, title)
        };
        let disabled = row.unavailable.is_some();
        let id = info.id();

        let chip = div()
            .flex_none()
            .px(u(tokens.spacing.sm))
            .rounded(u(tokens.radius.sm))
            .bg(colors.element)
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .child(highlighted_text(
                category.into(),
                &chip_hits,
                gpui::HighlightStyle {
                    color: Some(colors.accent),
                    ..Default::default()
                },
            ));
        let title = match_label(info.title().into(), &title_hits, selected, cx)
            .flex_1()
            .min_w_0()
            .truncate();
        let trailing: AnyElement = match &row.unavailable {
            Some(reason) => div()
                .flex_none()
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(reason.to_string())
                .into_any_element(),
            None => binding_caps(id.as_str(), format!("palette-keys-{ix}"), cx).into_any_element(),
        };
        Some(
            h_flex()
                .id(("palette-command", ix))
                .debug_selector(move || format!("palette-command-{ix}"))
                .w_full()
                .gap(u(tokens.spacing.md))
                .items_center()
                .when(disabled, |row| row.opacity(0.6))
                .child(chip)
                .child(title)
                .child(trailing),
        )
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Picker<Self>>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let hidden = self.snapshot.hidden();
        let summary = if self.show_all || hidden == 0 {
            format!("{} commands", self.found.len())
        } else {
            format!("{} commands, {hidden} unavailable hidden", self.found.len())
        };
        let on = self.show_all;
        h_flex()
            .id("palette-footer")
            .debug_selector(|| "palette-footer".into())
            .flex_none()
            .justify_between()
            .items_center()
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .border_t_1()
            .border_color(colors.border_variant)
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .child(
                div()
                    .debug_selector(|| "palette-summary".into())
                    .child(summary),
            )
            .child(
                h_flex()
                    .id("palette-show-all")
                    .debug_selector(|| "palette-show-all".into())
                    .gap(u(tokens.spacing.sm))
                    .items_center()
                    .cursor_pointer()
                    .rounded(u(tokens.radius.sm))
                    .px(u(tokens.spacing.sm))
                    .hover(|style| style.bg(colors.element_hover))
                    .text_color(if on { colors.text } else { colors.text_muted })
                    .on_click(cx.listener(|picker, _, window, cx| {
                        picker.toggle_show_all(window, cx);
                    }))
                    .child(
                        Icon::new(if on {
                            IconName::CircleCheck
                        } else {
                            IconName::Eye
                        })
                        .size(u(px(12.))),
                    )
                    .child("Show all")
                    .child(binding_caps(
                        TOGGLE_SHOW_ALL_ACTION,
                        "palette-show-all-keys".into(),
                        cx,
                    )),
            )
            .into_any_element()
    }
}

/// The keystrokes of the first binding of `action`, in the keymap's spelling (`cmd-shift-p`);
/// empty when it has none. Read from the live keymap, so a rebind in `keymap.json` shows up in
/// the next frame.
pub(super) fn binding_strokes(action: &str, cx: &gpui::App) -> Vec<String> {
    bindings_for_action_name(cx, action, None)
        .into_iter()
        .next()
        .map(|binding| binding.keystrokes)
        .unwrap_or_default()
}

/// The key caps of the first binding of `action` (none when it has none), selectable as
/// `selector` in tests.
fn binding_caps(action: &str, selector: String, cx: &gpui::App) -> gpui::Div {
    let caps = binding_strokes(action, cx)
        .iter()
        .filter_map(|stroke| keycap(stroke))
        .collect::<Vec<_>>();
    h_flex()
        .debug_selector(move || selector)
        .flex_none()
        .gap(u(px(2.)))
        .children(caps)
}
