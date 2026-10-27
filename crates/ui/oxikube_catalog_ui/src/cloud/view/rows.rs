//! The selected provider's setup banner, scope filter bar and cluster rows.

use gpui::{
    Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Styled as _,
    div, px,
};
use oxikube_domain::cloud::{CloudProvider, ClusterStatus};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::CloudView;
use crate::cloud::model::Phase;

/// The height of every cluster row, at 100 % zoom: uniform, as `uniform_list` needs.
pub(super) const ROW_HEIGHT: Pixels = px(44.);

/// "In catalog" marker text.
pub const IN_CATALOG: &str = "In catalog";

impl CloudView {
    /// What is wrong with the selected provider and what to do: the failed run, the detection's
    /// hints (log in, install a plugin) and the scopes that could not be listed.
    pub(super) fn banner(
        &self,
        provider: CloudProvider,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let state = self.model.provider(provider);
        let tokens = cx.tokens();
        let mut lines: Vec<(String, Option<String>)> = Vec::new();
        if let Some(error) = &state.error {
            lines.push((error.text.clone(), error.command.clone()));
        } else if state.phase() == Phase::Degraded || !state.hints().is_empty() {
            for hint in state.hints() {
                lines.push((hint.summary.clone(), hint.command.clone()));
            }
        }
        for failure in &state.failures {
            lines.push((
                format!("{}: {}", failure.scope, failure.remediation.text),
                failure.remediation.command.clone(),
            ));
        }
        if lines.is_empty() {
            return None;
        }
        let colour = if state.error.is_some() {
            tokens.colors.error
        } else {
            tokens.colors.warning
        };
        let rows: Vec<_> = lines
            .into_iter()
            .enumerate()
            .map(|(ix, (text, command))| {
                let copy = command.map(|command| {
                    div()
                        .debug_selector(move || format!("cloud-banner-copy-{ix}"))
                        .child(
                            Button::new(("cloud-banner-copy", ix))
                                .small()
                                .ghost()
                                .icon(Icon::new(IconName::Copy).size(u(px(14.))))
                                .label(format!("Copy `{command}`"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_command(&command, cx);
                                })),
                        )
                });
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .debug_selector(move || format!("cloud-banner-line-{ix}"))
                    .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
                    .child(div().flex_1().min_w_0().child(text))
                    .children(copy)
            })
            .collect();
        Some(
            v_flex()
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .mx(u(tokens.spacing.xl))
                .mb(u(tokens.spacing.md))
                .p(u(tokens.spacing.md))
                .rounded(u(tokens.radius.md))
                .text_size(u(tokens.font.small))
                .text_color(colour)
                .bg(colour.opacity(0.08))
                .debug_selector(|| "cloud-banner".into())
                .children(rows)
                .into_any_element(),
        )
    }

    /// One chip per scope of the listing (profile and region, project, subscription), and
    /// "All". Hidden while there is nothing to choose between.
    pub(super) fn filter_bar(
        &self,
        provider: CloudProvider,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let state = self.model.provider(provider);
        let scopes = state.scopes();
        if scopes.len() < 2 {
            return None;
        }
        let tokens = cx.tokens();
        let all = state.filter.is_none();
        let mut chips = vec![
            div().debug_selector(|| "cloud-filter-all".into()).child(
                Button::new("cloud-filter-all")
                    .small()
                    .ghost()
                    .selected(all)
                    .label("All")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.set_filter(provider, None);
                        cx.notify();
                    })),
            ),
        ];
        for (ix, scope) in scopes.into_iter().enumerate() {
            let selected = state.filter.as_ref() == Some(&scope);
            let label = scope.to_string();
            chips.push(
                div()
                    .debug_selector(move || format!("cloud-filter-{ix}"))
                    .child(
                        Button::new(("cloud-filter", ix))
                            .small()
                            .ghost()
                            .selected(selected)
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.model.set_filter(provider, Some(scope.clone()));
                                cx.notify();
                            })),
                    ),
            );
        }
        Some(
            h_flex()
                .flex_none()
                .flex_wrap()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.xl))
                .pb(u(tokens.spacing.md))
                .debug_selector(|| "cloud-filters".into())
                .children(chips)
                .into_any_element(),
        )
    }

    /// The list's stand-in when the provider has no cluster to show.
    pub(super) fn no_clusters(&self, provider: CloudProvider, cx: &mut Context<Self>) -> Div {
        let tokens = cx.tokens();
        let state = self.model.provider(provider);
        let text = match state.phase() {
            Phase::Loading { .. } => "Looking for clusters...".to_owned(),
            Phase::Error | Phase::Degraded => "No clusters to show yet.".to_owned(),
            _ if state.filter.is_some() => "No cluster in this scope.".to_owned(),
            _ => format!("No {} clusters found.", provider.service()),
        };
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(tokens.colors.text_muted)
            .debug_selector(|| "cloud-no-clusters".into())
            .child(text)
    }

    pub(super) fn render_row(
        &mut self,
        provider: CloudProvider,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(cluster) = self.model.provider(provider).shown().nth(ix).cloned() else {
            return div().into_any_element();
        };
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let added = self.model.added_as(&cluster).is_some();
        let armed = self.confirm_remove.as_deref() == Some(cluster.key().as_str());
        let busy = self.editing;
        let (status_colour, status_text) = status_badge(&cluster.status, &colors);
        let version = cluster
            .kubernetes_version
            .clone()
            .unwrap_or_else(|| "-".to_owned());

        let action = if added && armed {
            let remove = cluster.clone();
            h_flex()
                .gap(u(tokens.spacing.sm))
                .items_center()
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(colors.warning)
                        .child("Disconnects it if connected."),
                )
                .child(
                    div()
                        .debug_selector(move || format!("cloud-confirm-{ix}"))
                        .child(
                            Button::new(("cloud-confirm", ix))
                                .small()
                                .danger()
                                .label("Remove")
                                .disabled(busy)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.remove(&remove, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .debug_selector(move || format!("cloud-keep-{ix}"))
                        .child(
                            Button::new(("cloud-keep", ix))
                                .small()
                                .ghost()
                                .label("Keep")
                                .on_click(cx.listener(|this, _, _, cx| this.disarm_remove(cx))),
                        ),
                )
                .into_any_element()
        } else if added {
            let target = cluster.clone();
            div()
                .debug_selector(move || format!("cloud-remove-{ix}"))
                .child(
                    Button::new(("cloud-remove", ix))
                        .small()
                        .ghost()
                        .icon(Icon::new(IconName::Trash).size(u(px(14.))))
                        .label("Remove")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.arm_remove(&target, cx))),
                )
                .into_any_element()
        } else {
            let target = cluster.clone();
            div()
                .debug_selector(move || format!("cloud-add-{ix}"))
                .child(
                    Button::new(("cloud-add", ix))
                        .small()
                        .icon(Icon::new(IconName::Plus).size(u(px(14.))))
                        .label("Add")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.add(&target, cx))),
                )
                .into_any_element()
        };

        h_flex()
            .id(("cloud-row", ix))
            .debug_selector(move || format!("cloud-row-{ix}"))
            .h(u(ROW_HEIGHT))
            .w_full()
            .flex_none()
            .gap(u(tokens.spacing.lg))
            .px(u(tokens.spacing.xl))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(cluster.name.clone()),
            )
            .child(
                div()
                    .w(u(px(140.)))
                    .truncate()
                    .text_color(colors.text_muted)
                    .debug_selector(move || format!("cloud-location-{ix}"))
                    .child(cluster.location),
            )
            .child(
                div()
                    .w(u(px(110.)))
                    .truncate()
                    .text_color(colors.text_muted)
                    .debug_selector(move || format!("cloud-version-{ix}"))
                    .child(version),
            )
            .child(
                div()
                    .w(u(px(90.)))
                    .text_size(u(tokens.font.small))
                    .text_color(status_colour)
                    .debug_selector({
                        let selector = format!("cloud-status-{ix}:{status_text}");
                        move || selector
                    })
                    .child(status_text),
            )
            .child(
                div()
                    .w(u(px(80.)))
                    .text_size(u(tokens.font.small))
                    .text_color(colors.success)
                    .debug_selector(move || format!("cloud-in-catalog-{ix}:{added}"))
                    .child(if added { IN_CATALOG } else { "" }),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(u(px(110.)))
                    .flex()
                    .justify_end()
                    .child(action),
            )
            .into_any_element()
    }
}

fn status_badge(status: &ClusterStatus, colors: &oxikube_ui::Colors) -> (gpui::Hsla, String) {
    let colour = match status {
        ClusterStatus::Running => colors.success,
        ClusterStatus::Failed => colors.error,
        ClusterStatus::Creating | ClusterStatus::Updating | ClusterStatus::Deleting => {
            colors.warning
        }
        ClusterStatus::Stopped | ClusterStatus::Other(_) => colors.text_muted,
    };
    (colour, status.to_string())
}
