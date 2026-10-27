//! Drawing the binding panel: the header, the subjects table and the rules the role grants, both
//! virtualised.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use oxikube_domain::command::CommandId;
use oxikube_domain::rbac::Binding;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Sizable as _, Tokens, u};

use super::{BindingPanel, RolePhase};

/// The height of a row, at 100 % zoom: uniform, as the virtualised lists need.
const ROW_HEIGHT: f32 = 26.;

/// How many rows a table shows before it scrolls.
const VISIBLE_ROWS: usize = 8;

/// The subjects table: title and share of the width.
const SUBJECT_COLUMNS: [(&str, f32); 4] = [
    ("Kind", 1.),
    ("Name", 2.),
    ("Namespace", 1.),
    ("API group", 1.6),
];

/// The rules table: title and share of the width.
const RULE_COLUMNS: [(&str, f32); 5] = [
    ("API groups", 1.),
    ("Resources", 1.6),
    ("Verbs", 1.6),
    ("Resource names", 1.),
    ("Non-resource URLs", 1.),
];

impl BindingPanel {
    fn header(&self, binding: &Binding, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let edit = self.block(CommandId::BINDING_CREATE, cx);
        let delete = self.block(CommandId::BINDING_DELETE, cx);
        let role = format!(
            "{}/{}",
            binding.role_ref.kind.as_str(),
            binding.role_ref.name
        );
        let linkable = binding.role_id().is_some();
        let count = binding.subjects.len();
        h_flex()
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.lg))
            .pb(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.md))
            .items_center()
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .child(
                div()
                    .debug_selector(|| "binding-kind".to_owned())
                    .px(u(tokens.spacing.md))
                    .rounded(u(tokens.radius.sm))
                    .bg(colors.element)
                    .text_color(colors.text)
                    .child(binding.id.kind.as_str()),
            )
            .child("grants")
            .child(
                div()
                    .debug_selector(|| "binding-role-link".to_owned())
                    .child(
                        Button::new("binding-role-link")
                            .xsmall()
                            .ghost()
                            .label(role)
                            .disabled(!linkable)
                            .tooltip("Open the role")
                            .on_click(cx.listener(|this, _, _, cx| this.open_role(cx))),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "binding-subject-count".to_owned())
                    .child(match count {
                        1 => "to 1 subject".to_owned(),
                        n => format!("to {n} subjects"),
                    }),
            )
            .child(div().flex_1())
            .when_some(edit, |row, state| {
                row.child(
                    div().debug_selector(|| "binding-edit".to_owned()).child(
                        Button::new("binding-edit")
                            .xsmall()
                            .ghost()
                            .label("Edit…")
                            .disabled(state.is_some())
                            .tooltip(state.unwrap_or_else(|| {
                                "Change the role and the subjects; a new role replaces the binding"
                                    .into()
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.edit(cx))),
                    ),
                )
            })
            .when_some(delete, |row, state| {
                row.child(
                    div().debug_selector(|| "binding-delete".to_owned()).child(
                        Button::new("binding-delete")
                            .xsmall()
                            .ghost()
                            .label("Delete")
                            .disabled(state.is_some())
                            .tooltip(state.unwrap_or_else(|| {
                                "Delete the binding; its subjects lose what it grants".into()
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    ),
                )
            })
            .into_any_element()
    }

    fn heading(&self, text: String, id: &'static str, tokens: &Tokens) -> AnyElement {
        div()
            .debug_selector(move || id.to_owned())
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .text_size(u(tokens.font.small))
            .text_color(tokens.colors.text_muted)
            .child(text)
            .into_any_element()
    }

    fn header_row(
        &self,
        id: &'static str,
        columns: &[(&'static str, f32)],
        tokens: &Tokens,
    ) -> AnyElement {
        let colors = tokens.colors;
        h_flex()
            .debug_selector(move || id.to_owned())
            .h(u(px(ROW_HEIGHT)))
            .px(u(tokens.spacing.lg))
            .gap(u(tokens.spacing.md))
            .items_center()
            .border_b_1()
            .border_color(colors.border)
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .children(
                columns
                    .iter()
                    .map(|(title, share)| cell(*share, (*title).into())),
            )
            .into_any_element()
    }

    fn subject_row(&self, ix: usize, tokens: &Tokens) -> Option<AnyElement> {
        let row = self.subjects.get(ix)?;
        let colors = tokens.colors;
        let cells = [
            (SUBJECT_COLUMNS[0].1, row.kind.to_owned()),
            (SUBJECT_COLUMNS[1].1, row.name.to_string()),
            (SUBJECT_COLUMNS[2].1, row.namespace.to_string()),
            (SUBJECT_COLUMNS[3].1, row.api_group.to_owned()),
        ];
        Some(
            h_flex()
                .id(("binding-subject-row", ix))
                .debug_selector(move || format!("binding-subject-row-{ix}"))
                .h(u(px(ROW_HEIGHT)))
                .px(u(tokens.spacing.lg))
                .gap(u(tokens.spacing.md))
                .items_center()
                .text_size(u(tokens.font.small))
                .text_color(colors.text)
                .hover(|row| row.bg(colors.element))
                .children(cells.map(|(share, text)| cell(share, text)))
                .into_any_element(),
        )
    }

    fn rule_row(&self, ix: usize, tokens: &Tokens) -> Option<AnyElement> {
        let row = self.rules.get(ix)?;
        let colors = tokens.colors;
        let raw = row.raw.clone();
        let cells = [
            (RULE_COLUMNS[0].1, row.api_groups.to_string()),
            (RULE_COLUMNS[1].1, row.resources.to_string()),
            (RULE_COLUMNS[2].1, row.verbs.to_string()),
            (RULE_COLUMNS[3].1, row.resource_names.to_string()),
            (RULE_COLUMNS[4].1, row.non_resource_urls.to_string()),
        ];
        Some(
            h_flex()
                .id(("binding-rule", ix))
                .debug_selector(move || format!("binding-rule-{ix}"))
                .h(u(px(ROW_HEIGHT)))
                .px(u(tokens.spacing.lg))
                .gap(u(tokens.spacing.md))
                .items_center()
                .text_size(u(tokens.font.small))
                .text_color(colors.text)
                .hover(|row| row.bg(colors.element))
                .tooltip(move |window, cx| Tooltip::new(raw.to_string()).build(window, cx))
                .children(cells.map(|(share, text)| cell(share, text)))
                .into_any_element(),
        )
    }

    /// The rules the named role grants, or why there are none to show.
    fn rules_section(&self, binding: &Binding, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let role = format!(
            "{}/{}",
            binding.role_ref.kind.as_str(),
            binding.role_ref.name
        );
        let note = |text: String, id: &'static str| {
            div()
                .debug_selector(move || id.to_owned())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.sm))
                .text_color(colors.text_muted)
                .text_size(u(tokens.font.small))
                .child(text)
                .into_any_element()
        };
        let total = self.rules.len();
        match &self.phase {
            RolePhase::Loading => note(format!("Reading {role}…"), "binding-rules-loading"),
            RolePhase::Missing => note(
                format!("{role} does not exist, so this binding grants nothing"),
                "binding-rules-missing",
            ),
            RolePhase::Failed(why) => note(
                format!("Could not read {role}: {why}"),
                "binding-rules-failed",
            ),
            RolePhase::Ready if total == 0 => {
                note(format!("{role} has no rules"), "binding-rules-empty")
            }
            RolePhase::Ready => v_flex()
                .child(self.heading(
                    format!("Effective rules, from {role} ({total})"),
                    "binding-rules-title",
                    &tokens,
                ))
                .child(self.header_row("binding-rules-header", &RULE_COLUMNS, &tokens))
                .child(
                    div()
                        .debug_selector(|| "binding-rules".to_owned())
                        .h(u(px(ROW_HEIGHT * total.min(VISIBLE_ROWS) as f32)))
                        .w_full()
                        .child(
                            uniform_list(
                                "binding-rules-list",
                                total,
                                cx.processor(move |this, range: Range<usize>, _, cx| {
                                    let tokens = cx.tokens();
                                    range.filter_map(|ix| this.rule_row(ix, &tokens)).collect()
                                }),
                            )
                            .size_full(),
                        ),
                )
                .into_any_element(),
        }
    }
}

/// One cell: `share` of the row's width, one line, cut with an ellipsis.
fn cell(share: f32, text: String) -> AnyElement {
    div()
        .w(gpui::relative(share / 6.2))
        .min_w_0()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .child(text)
        .into_any_element()
}

impl Render for BindingPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let Some(binding) = self.binding.clone() else {
            return v_flex()
                .id("binding-panel")
                .debug_selector(|| "binding-panel".to_owned())
                .p(u(tokens.spacing.lg))
                .text_color(tokens.colors.text_muted)
                .text_size(u(tokens.font.small))
                .child("Reading the binding…")
                .into_any_element();
        };
        let total = self.subjects.len();
        v_flex()
            .id("binding-panel")
            .debug_selector(|| "binding-panel".to_owned())
            .child(self.header(&binding, cx))
            .child(if total == 0 {
                div()
                    .debug_selector(|| "binding-empty".to_owned())
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.md))
                    .text_color(tokens.colors.text_muted)
                    .child("This binding grants nothing: it has no subjects")
                    .into_any_element()
            } else {
                v_flex()
                    .child(self.header_row("binding-subjects-header", &SUBJECT_COLUMNS, &tokens))
                    .child(
                        div()
                            .debug_selector(|| "binding-subjects".to_owned())
                            .h(u(px(ROW_HEIGHT * total.min(VISIBLE_ROWS) as f32)))
                            .w_full()
                            .child(
                                uniform_list(
                                    "binding-subjects-list",
                                    total,
                                    cx.processor(move |this, range: Range<usize>, _, cx| {
                                        let tokens = cx.tokens();
                                        range
                                            .filter_map(|ix| this.subject_row(ix, &tokens))
                                            .collect()
                                    }),
                                )
                                .size_full(),
                            ),
                    )
                    .into_any_element()
            })
            .child(self.rules_section(&binding, cx))
            .into_any_element()
    }
}
