//! The frame of the metrics page.

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Colors, Icon, IconName, Sizable as _, u};

use super::MetricsSettingsView;
use crate::metrics::model::{Headline, ProbeLine, SettingLine, Tone};

/// The colour of `tone` in the active theme.
fn tone_colour(tone: Tone, colors: &Colors) -> Hsla {
    match tone {
        Tone::Neutral => colors.text,
        Tone::Good => colors.success,
        Tone::Pending => colors.info,
        Tone::Warning => colors.warning,
        Tone::Error => colors.error,
        Tone::Muted => colors.text_muted,
    }
}

/// The name a test selector carries for `tone`.
fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral => "neutral",
        Tone::Good => "good",
        Tone::Pending => "pending",
        Tone::Warning => "warning",
        Tone::Error => "error",
        Tone::Muted => "muted",
    }
}

impl Render for MetricsSettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let header = self.header(cx);
        let headline = self.headline_card(cx);
        let probes = self.probes_section(cx);
        let settings = self.settings_section(cx);
        v_flex()
            .id("metrics-settings")
            .debug_selector(|| "metrics-settings".into())
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(header)
            .child(
                v_flex()
                    .id("metrics-settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .gap(u(tokens.spacing.xl))
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.lg))
                    .child(headline)
                    .child(probes)
                    .child(settings),
            )
    }
}

impl MetricsSettingsView {
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let model = &self.model;
        let (busy, can_redetect, enabled) = (model.busy, model.can_redetect, model.enabled);
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.lg))
            .px(u(tokens.spacing.xl))
            .py(u(tokens.spacing.lg))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Metrics"),
            )
            .child(
                div()
                    .debug_selector(|| "metrics-cluster".into())
                    .text_color(colors.text_muted)
                    .child(SharedString::from(model.cluster_title.clone())),
            )
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "metrics-redetect".into()).child(
                    Button::new("metrics-redetect")
                        .small()
                        .icon(Icon::new(IconName::RefreshCw).size(u(px(14.))))
                        .label(if busy { "Looking..." } else { "Look again" })
                        .tooltip("Look for a Prometheus-compatible service in the cluster again")
                        .disabled(!can_redetect)
                        .on_click(cx.listener(|this, _, _, cx| this.redetect(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "metrics-toggle".into()).child(
                    Button::new("metrics-toggle")
                        .small()
                        .ghost()
                        .label(if enabled {
                            "Turn history metrics off"
                        } else {
                            "Turn history metrics on"
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_enabled(cx))),
                ),
            )
            .into_any_element()
    }

    fn headline_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Headline { text, tone } = self.model.headline.clone();
        let colour = tone_colour(tone, &colors);
        let icon = match tone {
            Tone::Good => IconName::CircleCheck,
            Tone::Pending => IconName::Activity,
            Tone::Warning | Tone::Error => IconName::CircleAlert,
            Tone::Neutral | Tone::Muted => IconName::ChartLine,
        };
        let selector = format!("metrics-headline:{}", tone_name(tone));
        let mut card = v_flex()
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.surface)
            .child(
                h_flex()
                    .debug_selector(move || selector)
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .text_color(colour)
                    .child(Icon::new(icon).size(u(px(16.))).color(colour))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(SharedString::from(text)),
                    ),
            );
        if let Some(reason) = self.model.unavailable.clone() {
            card = card.child(
                div()
                    .debug_selector(|| "metrics-unavailable".into())
                    .text_color(colors.text_muted)
                    .child(SharedString::from(reason)),
            );
        }
        if let Some(checked) = self.model.checked.clone() {
            card = card.child(
                div()
                    .debug_selector(|| "metrics-checked".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_disabled)
                    .child(SharedString::from(format!("Checked at {checked}"))),
            );
        }
        card.into_any_element()
    }

    fn probes_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let mut section = v_flex()
            .gap(u(tokens.spacing.sm))
            .child(section_title("Services looked at", cx))
            .child(
                div()
                    .debug_selector(|| "metrics-probes-note".into())
                    .text_color(colors.text_muted)
                    .child(SharedString::from(self.model.probes_note.clone())),
            );
        // The catalogue is a few dozen entries at most, so the rows are plain children.
        for (ix, probe) in self.model.probes.iter().enumerate() {
            section = section.child(probe_row(ix, probe, cx));
        }
        section.into_any_element()
    }

    fn settings_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let mut section = v_flex()
            .gap(u(tokens.spacing.sm))
            .child(section_title("Settings in effect", cx));
        for line in &self.model.settings {
            section = section.child(setting_row(line, cx));
        }
        section
            .child(
                div()
                    .debug_selector(|| "metrics-edit-hint".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(SharedString::from(self.model.edit_hint.clone())),
            )
            .into_any_element()
    }
}

fn section_title(title: &'static str, cx: &Context<MetricsSettingsView>) -> impl IntoElement {
    let tokens = cx.tokens();
    div()
        .text_size(u(tokens.font.heading))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(title)
}

fn probe_row(ix: usize, probe: &ProbeLine, cx: &Context<MetricsSettingsView>) -> impl IntoElement {
    let tokens = cx.tokens();
    let colors = tokens.colors;
    let colour = tone_colour(probe.tone, &colors);
    let selector = format!("metrics-probe-{ix}:{}", tone_name(probe.tone));
    h_flex()
        .id(("metrics-probe", ix))
        .debug_selector(move || selector)
        .gap(u(tokens.spacing.lg))
        .py(u(tokens.spacing.xs))
        .border_b_1()
        .border_color(colors.border_variant)
        .child(
            div()
                .w(u(px(320.)))
                .flex_none()
                .truncate()
                .child(SharedString::from(probe.service.clone())),
        )
        .child(
            div()
                .w(u(px(160.)))
                .flex_none()
                .truncate()
                .text_color(colors.text_muted)
                .child(SharedString::from(probe.stack.clone())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(colour)
                .child(SharedString::from(probe.outcome.clone())),
        )
}

fn setting_row(line: &SettingLine, cx: &Context<MetricsSettingsView>) -> impl IntoElement {
    let tokens = cx.tokens();
    let colors = tokens.colors;
    let selector = format!("metrics-setting-{}", line.label);
    h_flex()
        .debug_selector(move || selector)
        .gap(u(tokens.spacing.lg))
        .child(
            div()
                .w(u(px(160.)))
                .flex_none()
                .text_color(colors.text_muted)
                .child(line.label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(SharedString::from(line.value.clone())),
        )
}
