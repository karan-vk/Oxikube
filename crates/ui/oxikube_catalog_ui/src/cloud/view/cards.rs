//! One card per detected provider: name, CLI version, state, cluster count, refresh.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_domain::age::Age;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::CloudView;
use crate::cloud::model::{Phase, ProviderState};

/// How a provider's state reads on its card: "Ready", "Listing 3 of 7 scopes", "Needs setup".
pub fn phase_text(state: &ProviderState) -> String {
    match state.phase() {
        Phase::Idle => "Not checked yet".to_owned(),
        Phase::Loading { done, total } if total > 0 => format!("Listing {done} of {total} scopes"),
        Phase::Loading { .. } => "Looking for clusters...".to_owned(),
        Phase::Loaded => match state.clusters.len() {
            1 => "1 cluster".to_owned(),
            n => format!("{n} clusters"),
        },
        Phase::Degraded => "Needs setup".to_owned(),
        Phase::Error => "Could not list".to_owned(),
    }
}

impl CloudView {
    pub(super) fn cards(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = cx.tokens();
        let selected = self.model.selected().map(|s| s.provider);
        let now = self.deps.clock.now();
        let cards: Vec<_> = self
            .model
            .visible()
            .map(|state| {
                let provider = state.provider;
                let active = Some(provider) == selected;
                let colors = tokens.colors;
                let phase = state.phase();
                let colour = match phase {
                    Phase::Error => colors.error,
                    Phase::Degraded => colors.warning,
                    _ => colors.text_muted,
                };
                let version = state
                    .detection
                    .as_ref()
                    .and_then(|d| d.version.clone())
                    .map(|v| format!("{} {v}", provider.cli()))
                    .unwrap_or_else(|| provider.cli().to_owned());
                let age = state.fetched_at.map(|at| {
                    format!(
                        "refreshed {} ago",
                        Age::between(at, now).to_kubectl_string()
                    )
                });
                let meta = match age {
                    Some(age) => format!("{version} · {age}"),
                    None => version,
                };
                let meta_selector = format!("cloud-card-meta-{}:{meta}", provider.id());
                let loading = matches!(phase, Phase::Loading { .. });
                let status = phase_text(state);
                let status_selector = format!("cloud-card-status-{}:{status}", provider.id());
                div()
                    .id(("cloud-card", provider as usize))
                    .debug_selector(move || format!("cloud-card-{}", provider.id()))
                    .flex_1()
                    .min_w_0()
                    .cursor_pointer()
                    .p(u(tokens.spacing.lg))
                    .rounded(u(tokens.radius.md))
                    .border_1()
                    .border_color(if active {
                        colors.border_focused
                    } else {
                        colors.border
                    })
                    .bg(if active {
                        colors.element_selected
                    } else {
                        colors.surface
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.select(provider);
                        cx.notify();
                    }))
                    .child(
                        v_flex()
                            .gap(u(tokens.spacing.sm))
                            .child(
                                h_flex()
                                    .gap(u(tokens.spacing.md))
                                    .items_center()
                                    .child(Icon::new(IconName::Cloud).size(u(px(16.))))
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(provider.to_string()),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("cloud-card-refresh-{}", provider.id())
                                            })
                                            .child(
                                                Button::new(("cloud-refresh", provider as usize))
                                                    .small()
                                                    .ghost()
                                                    .icon(
                                                        Icon::new(IconName::RefreshCw)
                                                            .size(u(px(14.))),
                                                    )
                                                    .tooltip("Refresh")
                                                    .disabled(loading)
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.refresh_provider(provider, cx);
                                                        },
                                                    )),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap(u(tokens.spacing.md))
                                    .items_center()
                                    .text_size(u(tokens.font.small))
                                    .text_color(colour)
                                    .debug_selector(move || status_selector)
                                    .when(loading, |row| row.child(Spinner::new().small()))
                                    .child(status),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(u(tokens.font.small))
                                    .text_color(colors.text_muted)
                                    .debug_selector(move || meta_selector)
                                    .child(meta),
                            ),
                    )
            })
            .collect();
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xl))
            .children(cards)
            .into_any_element()
    }
}
