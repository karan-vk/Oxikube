//! The per-cluster scope's drawing: the scope switch, the cluster list and the precedence strip.

use std::ops::Range;

use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ListSizingBehavior, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, div, prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_settings::{SettingSource, SettingsStore};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Colors, u};

use super::precedence::{
    EDITS_ONLY_THE_CLUSTER, EXPLANATION, Precedence, PrecedenceCell, protection_warning,
};
use super::state::Scope;
use crate::page::{PageField, SettingsPage};

/// The tallest the cluster list grows before it scrolls, unscaled.
const LIST_HEIGHT: f32 = 200.0;

impl SettingsPage {
    /// The two buttons under the title.
    pub(in crate::page) fn scope_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let button = |scope: Scope, label: &'static str, selector: &'static str| {
            let selected = self.tab.scope == scope;
            div()
                .id(selector)
                .debug_selector(move || selector.to_owned())
                .flex_1()
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .rounded(u(tokens.radius.md))
                .cursor_pointer()
                .text_center()
                .when(selected, |button| {
                    button
                        .bg(colors.element_selected)
                        .font_weight(FontWeight::MEDIUM)
                })
                .hover(|style| style.bg(colors.element_hover))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.set_scope(scope, window, cx)))
        };
        h_flex()
            .gap(u(tokens.spacing.xs))
            .px(u(tokens.spacing.sm))
            .pb(u(tokens.spacing.md))
            .child(button(Scope::Global, "All", "settings-scope-global"))
            .child(button(
                Scope::Cluster,
                "Per cluster",
                "settings-scope-cluster",
            ))
            .into_any_element()
    }

    /// The precedence explanation and the cluster list: one element, the first of the content.
    /// The list is a `uniform_list` (only the rows in view are built and laid out, whatever the
    /// number of kubeconfig contexts) over the row texts worked out when the list or the file
    /// changed.
    pub(in crate::page) fn cluster_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let count = self.tab.rows.len();
        let list = (count > 0).then(|| {
            uniform_list(
                "settings-cluster-list",
                count,
                cx.processor(|this, range: Range<usize>, _, cx| this.cluster_list_rows(range, cx)),
            )
            // As tall as its rows, up to the cap; a scrolling column must not squeeze it.
            .with_sizing_behavior(ListSizingBehavior::Infer)
            .debug_selector(|| "settings-cluster-list".into())
            .w_full()
            .flex_none()
            .max_h(u(px(LIST_HEIGHT)))
            .into_any_element()
        });
        let empty = (count == 0).then(|| {
            div()
                .debug_selector(|| "settings-cluster-none".into())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.md))
                .text_color(colors.text_muted)
                .child("No clusters yet. Add a kubeconfig to configure a cluster.")
        });
        v_flex()
            .debug_selector(|| "settings-cluster-header".into())
            .gap(u(tokens.spacing.md))
            .pt(u(tokens.spacing.xl))
            .child(
                v_flex()
                    .debug_selector(|| "settings-precedence-explanation".into())
                    .gap(u(tokens.spacing.xs))
                    .p(u(tokens.spacing.lg))
                    .rounded(u(tokens.radius.md))
                    .border_1()
                    .border_color(colors.border_variant)
                    .bg(colors.surface)
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(
                        div()
                            .text_color(colors.text)
                            .font_weight(FontWeight::MEDIUM)
                            .child("Which value wins"),
                    )
                    .child(EXPLANATION)
                    .child(EDITS_ONLY_THE_CLUSTER),
            )
            .children(list)
            .children(empty)
            .into_any_element()
    }

    /// The rows of the cluster list in `range` (the ones in view).
    fn cluster_list_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let end = range
            .end
            .min(self.tab.choices.len())
            .min(self.tab.rows.len());
        let start = range.start.min(end);
        (start..end)
            .map(|ix| {
                let choice = &self.tab.choices[ix];
                let text = &self.tab.rows[ix];
                let selected = self.cluster.as_ref() == Some(&choice.id);
                let id = choice.id.clone();
                // Every row is the same height (two lines, the same padding), as the list needs;
                // the gap between rows is a bottom padding inside it.
                div()
                    .id(("settings-cluster", ix))
                    .pb(u(tokens.spacing.xs))
                    .child(
                        h_flex()
                            .debug_selector({
                                let id = id.clone();
                                move || format!("settings-cluster-{id}")
                            })
                            .gap(u(tokens.spacing.md))
                            .px(u(tokens.spacing.lg))
                            .py(u(tokens.spacing.sm))
                            .rounded(u(tokens.radius.md))
                            .cursor_pointer()
                            .when(selected, |row| row.bg(colors.element_selected))
                            .hover(|style| style.bg(colors.element_hover))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(text.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(u(tokens.font.small))
                                            .text_color(colors.text_muted)
                                            .child(text.detail.clone()),
                                    ),
                            )
                            .when(text.overrides > 0, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_size(u(tokens.font.small))
                                        .text_color(colors.warning)
                                        .child(match text.overrides {
                                            1 => "1 override".to_owned(),
                                            n => format!("{n} overrides"),
                                        }),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_cluster(&id, cx)))
                    .into_any_element()
            })
            .collect()
    }

    /// The strip under a field on the per-cluster tab: what each layer says, which wins, and a
    /// warning when the override weakens a protection. `None` on the global scope.
    pub(in crate::page) fn precedence_row(
        &self,
        field: &PageField,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.tab.scope != Scope::Cluster {
            return None;
        }
        let cluster = self.cluster.as_ref()?;
        let store = cx.try_global::<SettingsStore>()?;
        let values = store.layer_values(&field.path, Some(cluster));
        let precedence = Precedence::new(&values, store.field_source(&field.path, Some(cluster)));
        let warning = protection_warning(
            &field.path,
            store.effective_value(&field.path, None),
            values.cluster.as_ref(),
        );
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let path = field.path.clone();
        let cells = precedence
            .cells
            .iter()
            .map(|cell| precedence_cell(&path, cell, &colors, cx));
        Some(
            v_flex()
                .gap(u(tokens.spacing.xs))
                .child(
                    h_flex()
                        .debug_selector({
                            let path = path.clone();
                            move || format!("settings-precedence-{path}")
                        })
                        .gap(u(tokens.spacing.xs))
                        .children(cells),
                )
                .child(
                    div()
                        .debug_selector({
                            let path = path.clone();
                            move || {
                                format!(
                                    "settings-precedence-summary-{path}-{}",
                                    precedence.winner.label()
                                )
                            }
                        })
                        .text_size(u(tokens.font.small))
                        .text_color(layer_colour(precedence.winner, &colors))
                        .child(precedence.summary()),
                )
                .children(warning.map(|text| {
                    div()
                        .debug_selector(move || format!("settings-warning-{path}"))
                        .text_size(u(tokens.font.small))
                        .text_color(colors.warning)
                        .child(SharedString::from(format!("Warning: {text}")))
                }))
                .into_any_element(),
        )
    }
}

/// One layer of the strip; the winner is highlighted.
fn precedence_cell(
    path: &SharedString,
    cell: &PrecedenceCell,
    colors: &Colors,
    cx: &gpui::App,
) -> AnyElement {
    let tokens = cx.tokens();
    let layer = cell.layer;
    let path = path.clone();
    let winner = cell.winner;
    div()
        .debug_selector(move || {
            format!(
                "settings-layer-{path}-{}{}",
                layer.label(),
                if winner { "-winner" } else { "" }
            )
        })
        .flex_1()
        .min_w_0()
        .px(u(tokens.spacing.sm))
        .py(u(tokens.spacing.xs))
        .rounded(u(tokens.radius.sm))
        .border_1()
        .border_color(if winner {
            layer_colour(layer, colors)
        } else {
            colors.border_variant
        })
        .when(winner, |cell| cell.bg(colors.element_selected))
        .text_size(u(tokens.font.small))
        .child(
            h_flex()
                .gap(u(tokens.spacing.sm))
                .child(
                    div()
                        .flex_none()
                        .text_color(colors.text_muted)
                        .child(layer.label()),
                )
                .child(
                    div()
                        .truncate()
                        .when(winner, |value| value.font_weight(FontWeight::MEDIUM))
                        .child(cell.value.clone().unwrap_or_else(|| "\u{2014}".to_owned())),
                ),
        )
        .into_any_element()
}

fn layer_colour(source: SettingSource, colors: &Colors) -> Hsla {
    match source {
        SettingSource::Default => colors.text_muted,
        SettingSource::User => colors.accent,
        SettingSource::Cluster => colors.warning,
    }
}
