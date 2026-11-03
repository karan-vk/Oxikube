//! One field's row: its title and layer badge, the value in effect, its description, its path
//! and kind, then the control that edits it ([`crate::controls`], E21-S05). The shapes no control
//! covers show their value and say to edit it in `settings.json`.

use gpui::{
    AnyElement, App, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};
use oxikube_settings::{SettingSource, SettingsStore};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Colors, u};
use serde_json::Value;

use super::SettingsPage;
use crate::page::badge::LayerBadge;
use crate::page::model::PageField;

/// A value is cut to this many characters on its row.
const VALUE_CHARS: usize = 48;

impl SettingsPage {
    /// The row of `field`; `highlighted` when a search result jumped to it.
    pub(super) fn field_row(
        &self,
        field: &PageField,
        highlighted: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let source = self.source_of(&field.path, cx);
        let json_only = field.control.is_json_only();
        let value = json_only.then(|| self.value_text(&field.path, cx));
        let control = self
            .controls
            .render(field, source, self.cluster.as_ref(), cx);
        let precedence = self.precedence_row(field, cx);
        let mono = cx.mono_font_family();
        let path = field.path.clone();
        let kind = if json_only {
            format!("{}, edit in settings.json", field.kind.label())
        } else {
            field.kind.label().to_owned()
        };
        v_flex()
            .id(SharedString::from(format!("settings-field-{path}")))
            .debug_selector({
                let path = path.clone();
                move || format!("settings-field-{path}")
            })
            .gap(u(tokens.spacing.xs))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.lg))
            .border_b_1()
            .border_color(colors.border_variant)
            .when(highlighted, |row| {
                row.bg(colors.element_selected)
                    .border_l_2()
                    .border_color(colors.accent)
            })
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(field.title.clone()),
                    )
                    .child(badge(&path, LayerBadge::new(source), cx))
                    .child(div().flex_1())
                    .children(value.map(|value| {
                        div()
                            .debug_selector({
                                let path = path.clone();
                                move || format!("settings-value-{path}")
                            })
                            .max_w(u(gpui::px(320.)))
                            .truncate()
                            .font_family(mono.clone())
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(value)
                    })),
            )
            .when(!field.description.is_empty(), |row| {
                row.child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(colors.text_muted)
                        .child(field.description.clone()),
                )
            })
            .children(precedence)
            .children(control)
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_disabled)
                    .child(div().font_family(mono).child(path))
                    .child(div().child(kind)),
            )
            .into_any_element()
    }

    /// The value in effect for `path` as compact JSON, cut to [`VALUE_CHARS`]; `unset` when no
    /// layer gives one.
    fn value_text(&self, path: &str, cx: &App) -> SharedString {
        let value = cx
            .try_global::<SettingsStore>()
            .and_then(|store| store.effective_value(path, self.cluster.as_ref()));
        match value {
            None | Some(Value::Null) => "unset".into(),
            Some(value) => {
                let text = value.to_string();
                if text.chars().count() > VALUE_CHARS {
                    let cut: String = text.chars().take(VALUE_CHARS).collect();
                    format!("{cut}\u{2026}").into()
                } else {
                    text.into()
                }
            }
        }
    }
}

/// The layer badge of the field at `path`, with its tooltip.
fn badge(path: &SharedString, badge: LayerBadge, cx: &App) -> AnyElement {
    let tokens = cx.tokens();
    let colour = badge_colour(badge.source, &tokens.colors);
    let tooltip = SharedString::from(badge.tooltip());
    let path = path.clone();
    div()
        .id(SharedString::from(format!("settings-badge-{path}")))
        .debug_selector(move || format!("settings-badge-{path}-{}", badge.label()))
        .flex_none()
        .px(u(tokens.spacing.sm))
        .rounded(u(tokens.radius.sm))
        .border_1()
        .border_color(colour)
        .text_size(u(tokens.font.small))
        .text_color(colour)
        .when(badge.is_override(), |chip| {
            chip.font_weight(FontWeight::MEDIUM)
        })
        .child(badge.label())
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .into_any_element()
}

/// The colour of a badge for `source`: quiet for a default, the accent for the user's own value,
/// the warning colour for a cluster override (it differs from what other clusters see).
fn badge_colour(source: SettingSource, colors: &Colors) -> Hsla {
    match source {
        SettingSource::Default => colors.text_muted,
        SettingSource::User => colors.accent,
        SettingSource::Cluster => colors.warning,
    }
}
