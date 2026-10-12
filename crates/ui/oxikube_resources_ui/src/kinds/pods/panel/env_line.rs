//! The environment line of the pod panel: a variable's name, its value (masked when it is a
//! Secret's), the reveal toggle and where it comes from.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

use super::PodPanel;
use super::env_rows::EnvCell;
use super::model::{EnvKey, EnvLine};
use super::render::{line, name_cell, value_cell};
use crate::table::ToneColors;
use oxikube_domain::view::{MASK, SecretValue};

impl PodPanel {
    /// The text the environment line draws for the Secret value `value` of variable `key`: the
    /// mask until the viewer reveals that variable, then the decoded value.
    pub(super) fn secret_text<'a>(&self, key: &EnvKey, value: &'a SecretValue) -> &'a str {
        if self.revealed.contains(key) {
            value.reveal()
        } else {
            MASK
        }
    }

    pub(super) fn env_line(
        &self,
        section: &'static str,
        ix: usize,
        env: &EnvLine,
        tokens: &Tokens,
        tones: &ToneColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let EnvLine {
            key,
            name,
            cell,
            origin,
        } = env;
        let muted = tokens.colors.text_muted;
        let mono = cx.mono_font_family();
        let revealed = self.revealed.contains(key);
        let secret = matches!(cell, EnvCell::Secret(_));
        let value: AnyElement = match cell {
            EnvCell::Plain(text) => value_cell(text, tokens.colors.text)
                .font_family(mono)
                .into_any_element(),
            EnvCell::Secret(value) => value_cell(self.secret_text(key, value), tokens.colors.text)
                .debug_selector(move || format!("pod-env-{ix}-value"))
                .font_family(mono)
                .into_any_element(),
            EnvCell::Chip { text, tone } => {
                let color = tones.of(*tone);
                div()
                    .debug_selector(move || format!("pod-env-{ix}-chip"))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .px(u(tokens.spacing.sm))
                            .rounded(u(tokens.radius.sm))
                            .bg(color.opacity(0.14))
                            .text_color(color)
                            .truncate()
                            .child(text.clone()),
                    )
                    .into_any_element()
            }
        };
        let reveal = secret.then(|| {
            let key = key.clone();
            div()
                .flex_none()
                .debug_selector(move || format!("pod-env-{ix}-reveal"))
                .child(
                    Button::new(("pod-env-reveal", ix))
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(if revealed {
                            IconName::EyeOff
                        } else {
                            IconName::Eye
                        }))
                        .tooltip(if revealed {
                            "Hide the value"
                        } else {
                            "Show the value (kept in memory only)"
                        })
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_reveal(key.clone(), cx)),
                        ),
                )
        });
        line(tokens, section, ix)
            .child(
                name_cell(name, tokens.colors.text)
                    .debug_selector(move || format!("pod-env-{ix}-name"))
                    .font_family(cx.mono_font_family()),
            )
            .child(value)
            .children(reveal)
            .child(
                div()
                    .flex_none()
                    .max_w(u(px(220.)))
                    .truncate()
                    .text_size(u(tokens.font.small))
                    .text_color(muted)
                    .child(origin.clone()),
            )
            .into_any_element()
    }
}
