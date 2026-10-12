//! Drawing the workload panel.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_app::columns::Tone;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

use super::WorkloadPanel;
use super::model::{ContainerLine, PanelModel};
use super::owned::{Owned, OwnedRow};
use crate::table::ToneColors;

/// The tallest the panel gets before it scrolls, unscaled pixels: the generic rows below keep the
/// room.
const MAX_HEIGHT: f32 = 300.;
/// The width of the key column, unscaled pixels.
const KEY_WIDTH: f32 = 110.;

fn heading(tokens: &Tokens, id: &'static str, text: String) -> AnyElement {
    div()
        .debug_selector(move || id.into())
        .pt(u(tokens.spacing.md))
        .text_size(u(tokens.font.small))
        .font_semibold()
        .text_color(tokens.colors.text_muted)
        .child(text.to_uppercase())
        .into_any_element()
}

fn line(tokens: &Tokens, id: &'static str, key: &'static str, value: String) -> AnyElement {
    h_flex()
        .debug_selector(move || id.into())
        .items_start()
        .gap(u(tokens.spacing.md))
        .child(
            div()
                .flex_none()
                .w(u(px(KEY_WIDTH)))
                .text_color(tokens.colors.text_muted)
                .child(key),
        )
        .child(div().flex_1().min_w_0().child(value))
        .into_any_element()
}

fn container(tokens: &Tokens, index: usize, c: &ContainerLine) -> AnyElement {
    let detail = [c.ports.as_str(), c.resources.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    v_flex()
        .debug_selector(move || format!("workload-container-{index}"))
        .child(
            h_flex()
                .gap(u(tokens.spacing.md))
                .child(div().font_semibold().child(c.name.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(tokens.colors.text_muted)
                        .child(c.image.to_string()),
                ),
        )
        .children((!detail.is_empty()).then(|| {
            div()
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child(detail)
        }))
        .into_any_element()
}

impl WorkloadPanel {
    fn stats_row(&self, tokens: &Tokens, tones: &ToneColors, model: &PanelModel) -> AnyElement {
        h_flex()
            .debug_selector(|| "workload-stats".into())
            .flex_wrap()
            .gap_x(u(tokens.spacing.lg))
            .gap_y(u(px(2.)))
            .children(model.stats.iter().enumerate().map(|(index, stat)| {
                let color = match stat.tone {
                    Tone::Neutral => tokens.colors.text,
                    tone => tones.of(tone),
                };
                h_flex()
                    .debug_selector(move || format!("workload-stat-{index}"))
                    .gap(u(px(4.)))
                    .child(div().text_color(tokens.colors.text_muted).child(stat.label))
                    .child(
                        div()
                            .font_semibold()
                            .text_color(color)
                            .child(stat.value.clone()),
                    )
            }))
            .into_any_element()
    }

    fn scale_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let blocked = self.scale_state(cx)?;
        let tooltip: SharedString = blocked
            .clone()
            .unwrap_or_else(|| "Set the number of replicas".to_owned())
            .into();
        Some(
            div()
                .debug_selector(|| "workload-scale".into())
                .child(
                    Button::new("workload-scale")
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(IconName::Scaling).size(u(px(14.))))
                        .label("Scale")
                        .tooltip(tooltip)
                        .disabled(blocked.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.open_scale(window, cx))),
                )
                .into_any_element(),
        )
    }

    /// The Rollback button (E12-S06): opens the rollout history dialog. Only for the kinds that
    /// keep a history, disabled with the reason on a read-only cluster.
    fn rollback_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let blocked = self.rollback_state(cx)?;
        let tooltip: SharedString = blocked
            .clone()
            .unwrap_or_else(|| "Show the rollout history and roll back to a revision".to_owned())
            .into();
        Some(
            div()
                .debug_selector(|| "workload-rollback".into())
                .child(
                    Button::new("workload-rollback")
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(IconName::RotateCcw).size(u(px(14.))))
                        .label("Rollback")
                        .tooltip(tooltip)
                        .disabled(blocked.is_some())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_rollback(window, cx)),
                        ),
                )
                .into_any_element(),
        )
    }

    fn owned_section(
        &self,
        tokens: &Tokens,
        tones: &ToneColors,
        id: &'static str,
        title: &'static str,
        owned: &Owned,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if owned.total == 0 {
            return Vec::new();
        }
        let mut out = vec![heading(tokens, id, format!("{title} ({})", owned.total))];
        for (index, row) in owned.rows.iter().enumerate() {
            out.push(self.owned_row(tokens, tones, id, index, row, cx));
        }
        if owned.hidden() > 0 {
            out.push(
                div()
                    .text_color(tokens.colors.text_muted)
                    .child(format!("and {} more", owned.hidden()))
                    .into_any_element(),
            );
        }
        out
    }

    fn owned_row(
        &self,
        tokens: &Tokens,
        tones: &ToneColors,
        section: &'static str,
        index: usize,
        row: &OwnedRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = row.target.clone();
        let color = match row.tone {
            Tone::Neutral => tokens.colors.text_muted,
            tone => tones.of(tone),
        };
        h_flex()
            .id((section, index))
            .debug_selector(move || format!("{section}-row-{index}"))
            .gap(u(tokens.spacing.md))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.open_object(target.clone(), cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(tokens.colors.accent)
                    .child(row.target.name.to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(color)
                    .child(row.status.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(u(px(64.)))
                    .text_right()
                    .text_color(tokens.colors.text_muted)
                    .child(row.info.clone()),
            )
            .into_any_element()
    }
}

impl Render for WorkloadPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let Some(model) = self.model.clone() else {
            return div().debug_selector(|| "workload-panel-empty".into());
        };
        let mut body = v_flex()
            .id("workload-panel")
            .debug_selector(|| "workload-panel".into())
            .max_h(u(px(MAX_HEIGHT)))
            .overflow_y_scroll()
            .gap(u(px(3.)))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(tokens.colors.border)
            .text_size(u(tokens.font.small))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(heading(&tokens, "workload-replicas", "Replicas".to_owned()))
                    .child(
                        h_flex()
                            .gap(u(tokens.spacing.sm))
                            .children(self.rollback_button(cx))
                            .children(self.scale_button(cx)),
                    ),
            )
            .child(self.stats_row(&tokens, &tones, &model));
        if let Some(strategy) = &model.strategy {
            body = body.child(line(
                &tokens,
                "workload-strategy",
                "Strategy",
                strategy.clone(),
            ));
        }
        if !model.selector.is_empty() {
            body = body.child(line(
                &tokens,
                "workload-selector",
                "Selector",
                model.selector.clone(),
            ));
        }
        for (key, value) in &model.extras {
            body = body.child(line(&tokens, "workload-extra", key, value.clone()));
        }
        if !model.containers.is_empty() {
            body = body.child(heading(
                &tokens,
                "workload-template",
                "Pod template".to_owned(),
            ));
            for (index, c) in model.containers.iter().enumerate() {
                body = body.child(container(&tokens, index, c));
            }
        }
        let replicasets = self.replicaset_rows.clone();
        let pods = self.pod_rows.clone();
        for element in self.owned_section(
            &tokens,
            &tones,
            "workload-replicasets",
            "ReplicaSets",
            &replicasets,
            cx,
        ) {
            body = body.child(element);
        }
        for element in self.owned_section(&tokens, &tones, "workload-pods", "Pods", &pods, cx) {
            body = body.child(element);
        }
        div().w_full().child(body)
    }
}
