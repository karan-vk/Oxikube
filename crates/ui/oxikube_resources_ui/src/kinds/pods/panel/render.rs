//! Drawing the pod panel: the action bar, the containers sub-table and the sections, each row one
//! line of one height, so a long list scrolls without measuring.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::button::Button;
use oxikube_ui::layout::{Disableable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

use super::PodPanel;
use super::actions::{BAR, command_for};
use super::model::{Row, Section};
use crate::table::ToneColors;

/// One row's height, unscaled: every row is one line.
const ROW_HEIGHT: f32 = 24.;
/// The tallest the sections grow before they scroll inside the drawer, unscaled pixels.
const MAX_HEIGHT: f32 = 360.;
/// The width of a label or a variable name, unscaled pixels.
const NAME_WIDTH: f32 = 190.;

impl Render for PodPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let bar = self.action_bar(&tokens, cx);
        let sections: Vec<AnyElement> = self
            .sections
            .iter()
            .map(|section| self.section(section, &tokens, &tones, cx))
            .collect();
        v_flex()
            .debug_selector(|| "pod-panel".to_owned())
            .w_full()
            .child(bar)
            .child(
                // The containers and the sections scroll together under the fixed bar, so the
                // panel never takes more than MAX_HEIGHT from the Overview below it.
                v_flex()
                    .id("pod-sections")
                    .debug_selector(|| "pod-sections".to_owned())
                    .w_full()
                    .max_h(u(px(MAX_HEIGHT)))
                    .overflow_y_scroll()
                    .pb(u(tokens.spacing.sm))
                    .border_b_1()
                    .border_color(tokens.colors.border_variant)
                    .child(self.containers.clone())
                    .children(sections),
            )
    }
}

impl PodPanel {
    fn action_bar(&self, tokens: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let detail = self.detail.upgrade();
        let buttons: Vec<AnyElement> = BAR
            .iter()
            .filter_map(|action| {
                // `None`: the session may not run it at all; `Some(why)`: blocked right now.
                let blocked = detail.as_ref()?.read(cx).command_block(action.id)?;
                let tip = blocked.clone().unwrap_or_else(|| action.tip.to_owned());
                let (id, key) = (action.id, action.key);
                Some(
                    div()
                        .debug_selector(move || format!("pod-action-{key}"))
                        .child(
                            Button::new(SharedString::from(format!("pod-action-{key}")))
                                .xsmall()
                                .label(action.label)
                                .tooltip(tip)
                                .disabled(blocked.is_some())
                                .on_click(cx.listener(move |this, _, _, cx| this.send(id, cx))),
                        )
                        .into_any_element(),
                )
            })
            .collect();
        h_flex()
            .id("pod-actions")
            .debug_selector(|| "pod-actions".to_owned())
            .flex_wrap()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .children(buttons)
            .into_any_element()
    }

    /// Sends the command of the bar's button `id` for this pod.
    fn send(&mut self, id: CommandId, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let pod = detail.read(cx).target().clone();
        if let Some(command) = command_for(id, &pod) {
            detail.update(cx, |detail, cx| detail.dispatch_command(command, cx));
        }
    }

    fn section(
        &self,
        section: &Section,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = section.id;
        let key = id.key();
        let open = self.open.contains(&id);
        let header = h_flex()
            .id(SharedString::from(format!("pod-section-{key}")))
            .debug_selector(move || format!("pod-section-{key}"))
            .h(u(px(ROW_HEIGHT)))
            .gap(u(tokens.spacing.sm))
            .items_center()
            .px(u(tokens.spacing.lg))
            .mt(u(tokens.spacing.sm))
            .cursor_pointer()
            .hover(|style| style.bg(tokens.colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(id, cx)))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(u(px(14.)))
                .color(tokens.colors.text_muted),
            )
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .font_semibold()
                    .text_color(tokens.colors.text_muted)
                    .child(id.title().to_uppercase()),
            )
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text_muted)
                    .child(format!("({})", section.count)),
            );
        let rows = open.then(|| {
            section
                .rows
                .iter()
                .enumerate()
                .map(|(ix, row)| self.row(key, ix, row, tokens, tones, cx))
                .collect::<Vec<_>>()
        });
        v_flex()
            .child(header)
            .children(rows.into_iter().flatten())
            .into_any_element()
    }

    fn row(
        &self,
        section: &'static str,
        ix: usize,
        row: &Row,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = tokens.colors.text_muted;
        match row {
            Row::Pair { label, value } => line(tokens, section, ix)
                .child(name_cell(label, muted))
                .child(value_cell(value, tokens.colors.text))
                .into_any_element(),
            Row::Link {
                label,
                text,
                target,
            } => {
                let target = target.clone();
                line(tokens, section, ix)
                    .child(name_cell(label, muted))
                    .child(
                        div()
                            .id(SharedString::from(format!("pod-link-{section}-{ix}")))
                            .debug_selector(move || format!("pod-link-{section}-{ix}"))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .cursor_pointer()
                            .text_color(tokens.colors.accent)
                            .hover(|style| style.underline())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_target(target.clone(), cx)
                            }))
                            .child(text.clone()),
                    )
                    .into_any_element()
            }
            Row::Env(env) => self.env_line(section, ix, env, tokens, tones, cx),
            Row::Head(text) => div()
                .h(u(px(ROW_HEIGHT)))
                .px(u(tokens.spacing.lg))
                .pt(u(tokens.spacing.xs))
                .font_semibold()
                .child(text.clone())
                .into_any_element(),
            Row::Cells(cells) => {
                let mut row = line(tokens, section, ix);
                for (col, cell) in cells.iter().enumerate() {
                    row = row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when(col > 0, |c| c.text_color(muted))
                            .child(cell.clone()),
                    );
                }
                row.into_any_element()
            }
            Row::Note(text) => line(tokens, section, ix)
                .text_color(muted)
                .text_size(u(tokens.font.small))
                .child(text.clone())
                .into_any_element(),
        }
    }

    /// Opens the detail of `target`.
    fn open_target(&mut self, target: ResourceRef, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail.upgrade() {
            detail.update(cx, |detail, cx| detail.open_resource(target, cx));
        }
    }
}

/// A row: one line of [`ROW_HEIGHT`], padded like the Overview's.
pub(super) fn line(tokens: &Tokens, section: &'static str, ix: usize) -> Div {
    h_flex()
        .debug_selector(move || format!("pod-row-{section}-{ix}"))
        .h(u(px(ROW_HEIGHT)))
        .gap(u(tokens.spacing.md))
        .items_center()
        .px(u(tokens.spacing.lg))
}

pub(super) fn name_cell(text: &str, color: gpui::Hsla) -> Div {
    div()
        .flex_none()
        .w(u(px(NAME_WIDTH)))
        .truncate()
        .text_color(color)
        .child(text.to_owned())
}

pub(super) fn value_cell(text: &str, color: gpui::Hsla) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_color(color)
        .child(text.to_owned())
}
