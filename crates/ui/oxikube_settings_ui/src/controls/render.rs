//! The control elements of a field: the control itself, the inline error and the reset button.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px, rgb,
};
use oxikube_domain::ClusterColour;
use oxikube_domain::ids::ClusterId;
use oxikube_settings::SettingSource;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::switch::Switch;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use serde_json::Value;

use super::spec::ControlSpec;
use super::state::{Controls, effective_value};
use crate::page::{PageField, SettingsPage};

/// Width of a text-like control, unscaled.
const INPUT_WIDTH: f32 = 280.0;
/// How many installed fonts the picker lists at once.
const FONT_SUGGESTIONS: usize = 6;

impl Controls {
    /// The control of `field`, its inline error and its reset button; `None` for the shapes that
    /// are edited in `settings.json`.
    pub(crate) fn render(
        &self,
        field: &PageField,
        source: SettingSource,
        cluster: Option<&ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) -> Option<AnyElement> {
        let control = match &field.control {
            ControlSpec::Switch => Some(self.switch(field, cluster, cx)),
            ControlSpec::Choice(values) => Some(self.choice(field, values, cluster, cx)),
            ControlSpec::Number(_) | ControlSpec::Text => {
                Some(self.input_box(field)?.into_any_element())
            }
            ControlSpec::Color => self.colour(field, cx),
            ControlSpec::Font => self.font(field, cx),
            ControlSpec::List => self.list(field, cx),
            ControlSpec::Json => return None,
        }?;
        let tokens = cx.tokens();
        let path = field.path.clone();
        let overridden = match cluster {
            Some(_) => source == SettingSource::Cluster,
            None => source == SettingSource::User,
        };
        let reset = overridden.then(|| {
            let target = path.clone();
            div()
                .debug_selector({
                    let path = path.clone();
                    move || format!("settings-reset-{path}")
                })
                .child(
                    Button::new(SharedString::from(format!("settings-reset-{path}")))
                        .xsmall()
                        .ghost()
                        .label(if cluster.is_some() {
                            "Reset for this cluster"
                        } else {
                            "Reset to default"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let cluster = this.cluster.clone();
                            this.controls.reset(&target, cluster, cx);
                        })),
                )
        });
        let error = self.error(&field.path).map(|error| {
            let path = path.clone();
            div()
                .debug_selector(move || format!("settings-error-{path}"))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.error)
                .child(error.to_string())
        });
        Some(
            v_flex()
                .gap(u(tokens.spacing.xs))
                .child(
                    h_flex()
                        .gap(u(tokens.spacing.md))
                        .items_start()
                        .child(control)
                        .children(reset),
                )
                .children(error)
                .into_any_element(),
        )
    }

    fn marked(path: &SharedString, prefix: &str, element: impl IntoElement) -> AnyElement {
        let selector = format!("{prefix}-{path}");
        div()
            .debug_selector(move || selector)
            .child(element)
            .into_any_element()
    }

    fn switch(
        &self,
        field: &PageField,
        cluster: Option<&ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) -> AnyElement {
        let on = effective_value(&field.path, cluster, cx)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let path = field.path.clone();
        let switch = Switch::new(SharedString::from(format!("settings-switch-{path}")))
            .checked(on)
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                let cluster = this.cluster.clone();
                this.controls
                    .chose(&path, Value::Bool(*checked), cluster, cx);
            }));
        Self::marked(&field.path, "settings-control", switch)
    }

    fn choice(
        &self,
        field: &PageField,
        values: &[SharedString],
        cluster: Option<&ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let current = effective_value(&field.path, cluster, cx);
        let current = current.as_ref().and_then(Value::as_str).map(str::to_owned);
        let buttons = values.iter().map(|value| {
            let selected = current.as_deref() == Some(value.as_ref());
            let path = field.path.clone();
            let chosen = value.clone();
            let button = Button::new(SharedString::from(format!(
                "settings-choice-{}-{value}",
                field.path
            )))
            .xsmall()
            .label(value.clone());
            let button = if selected {
                button.primary()
            } else {
                button.outline()
            };
            let selector = format!("settings-choice-{}-{value}", field.path);
            div()
                .debug_selector(move || selector)
                .child(button.on_click(cx.listener(move |this, _, _, cx| {
                    let cluster = this.cluster.clone();
                    this.controls
                        .chose(&path, Value::String(chosen.to_string()), cluster, cx);
                })))
        });
        h_flex()
            .flex_wrap()
            .gap(u(tokens.spacing.sm))
            .children(buttons)
            .into_any_element()
    }

    fn input_box(&self, field: &PageField) -> Option<gpui::Div> {
        let input = self.input(&field.path)?;
        let selector = format!("settings-input-{}", field.path);
        Some(
            div()
                .debug_selector(move || selector)
                .w(u(px(INPUT_WIDTH)))
                .child(Input::new(input).small()),
        )
    }

    fn colour(&self, field: &PageField, cx: &mut Context<SettingsPage>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let typed = self.input(&field.path)?.read(cx).value().to_string();
        let swatch = ClusterColour::parse(&typed).ok().map(|colour| {
            let packed = u32::from(colour.r) << 16 | u32::from(colour.g) << 8 | u32::from(colour.b);
            div()
                .debug_selector({
                    let path = field.path.clone();
                    move || format!("settings-swatch-{path}")
                })
                .flex_none()
                .size(u(px(20.)))
                .rounded(u(tokens.radius.sm))
                .border_1()
                .border_color(tokens.colors.border)
                .bg(rgb(packed))
        });
        Some(
            h_flex()
                .gap(u(tokens.spacing.md))
                .children(swatch)
                .child(self.input_box(field)?)
                .into_any_element(),
        )
    }

    fn font(&self, field: &PageField, cx: &mut Context<SettingsPage>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let typed = self.input(&field.path)?.read(cx).value().to_lowercase();
        let fonts = self.fonts.as_deref().unwrap_or_default();
        let exact = fonts.iter().any(|font| font.to_lowercase() == typed);
        let suggestions: Vec<SharedString> = if typed.is_empty() || exact {
            Vec::new()
        } else {
            fonts
                .iter()
                .filter(|font| font.to_lowercase().contains(&typed))
                .take(FONT_SUGGESTIONS)
                .cloned()
                .collect()
        };
        let rows = suggestions.into_iter().enumerate().map(|(ix, font)| {
            let path = field.path.clone();
            let chosen = font.clone();
            div()
                .id(("settings-font", ix))
                .debug_selector({
                    let path = field.path.clone();
                    move || format!("settings-font-{path}-{ix}")
                })
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .cursor_pointer()
                .hover(|style| style.bg(tokens.colors.element_hover))
                .child(font)
                .on_click(cx.listener(move |this, _, window, cx| {
                    // `set_value` does not emit a change: the write is started here.
                    if let Some(input) = this.controls.input(&path).cloned() {
                        input.update(cx, |input, cx| {
                            input.set_value(chosen.to_string(), window, cx);
                        });
                        let cluster = this.cluster.clone();
                        this.controls
                            .typed(&path, &ControlSpec::Font, &chosen, cluster, cx);
                    }
                }))
        });
        Some(
            v_flex()
                .child(self.input_box(field)?)
                .child(v_flex().w(u(px(INPUT_WIDTH))).children(rows))
                .into_any_element(),
        )
    }
}
