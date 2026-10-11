//! [`SecretPanel`]: a Secret's type, keys and the buttons that reveal or edit them.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, WeakEntity, Window, div, px, uniform_list,
};
use oxikube_app::SecretSummary;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::facts::fact_rows;
use super::rows::{Cell, key_row};
use crate::detail::{DetailView, RevealLoad};
use crate::kinds::key_list;
use crate::table::ToneColors;

/// The Secret panel. See the [module docs](super).
pub struct SecretPanel {
    detail: WeakEntity<DetailView>,
    _detail: Option<Subscription>,
}

impl SecretPanel {
    /// A panel over `detail`, redrawn when it changes.
    pub fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let subscription = detail
            .upgrade()
            .map(|entity| cx.observe(&entity, |_, _, cx| cx.notify()));
        Self {
            detail,
            _detail: subscription,
        }
    }

    fn toggle_key(&self, key: &str, cx: &mut Context<Self>) {
        let key = key.to_owned();
        self.detail
            .update(cx, |detail, cx| detail.request_reveal(Some(&key), cx))
            .ok();
    }

    fn toggle_all(&self, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.request_reveal(None, cx))
            .ok();
    }

    fn edit_decoded(&self, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.request_edit_decoded(cx))
            .ok();
    }

    /// The header: type, key count, immutable, and the two buttons.
    fn header(
        &self,
        summary: &SecretSummary,
        requested: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let keys = summary.keys.len();
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
                    .debug_selector(|| "secret-type".to_owned())
                    .px(u(tokens.spacing.md))
                    .rounded(u(tokens.radius.sm))
                    .bg(colors.element)
                    .text_color(colors.text)
                    .child(summary.secret_type.label().to_owned()),
            )
            .child(
                div()
                    .debug_selector(|| "secret-count".to_owned())
                    .child(match keys {
                        1 => "1 key".to_owned(),
                        n => format!("{n} keys"),
                    }),
            )
            .when(summary.immutable, |row| row.child("immutable"))
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "secret-reveal".to_owned()).child(
                    Button::new("secret-reveal")
                        .xsmall()
                        .ghost()
                        .icon(
                            Icon::new(if requested { IconName::EyeOff } else { IconName::Eye })
                                .size(u(px(14.))),
                        )
                        .label(if requested { "Hide values" } else { "Reveal values" })
                        .toggled(requested)
                        .tooltip(if requested {
                            "Hide the values (x)"
                        } else {
                            "Show the values as text, in memory only (x)"
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_all(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "secret-edit-decoded".to_owned()).child(
                    Button::new("secret-edit-decoded")
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(IconName::Pencil).size(u(px(14.))))
                        .label("Edit decoded")
                        .tooltip("Edit the values as text; they are base64-encoded when applied (shift-e)")
                        .on_click(cx.listener(|this, _, _, cx| this.edit_decoded(cx))),
                ),
            )
            .into_any_element()
    }
}

impl Render for SecretPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let Some(detail) = self.detail.upgrade() else {
            return div().into_any_element();
        };
        let (summary, requested, failure, now) = {
            let detail = detail.read(cx);
            let failure = match detail.reveal_load() {
                RevealLoad::Failed(message) => Some(message.clone()),
                _ => None,
            };
            (
                detail.secret_summary().cloned(),
                detail.reveal_requested(),
                failure,
                detail.current_time(),
            )
        };
        let Some(summary) = summary else {
            return key_list::reading(&tokens, "secret-panel", "Secret");
        };
        let tones = ToneColors::current(cx);
        let facts = fact_rows(&tokens, &tones, &summary, now);
        let total = summary.keys.len();
        let rows: Arc<SecretSummary> = summary.clone();
        v_flex()
            .id("secret-panel")
            .debug_selector(|| "secret-panel".to_owned())
            .child(self.header(&summary, requested, cx))
            .children(facts)
            .children(failure.map(|message| {
                div()
                    .debug_selector(|| "secret-reveal-error".to_owned())
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .text_size(u(tokens.font.small))
                    .text_color(tones.error)
                    .child(format!("Could not reveal the values: {message}"))
            }))
            .child(if total == 0 {
                key_list::empty(&tokens, "secret-empty", "No keys")
            } else {
                div()
                    .debug_selector(|| "secret-keys".to_owned())
                    .mt(u(tokens.spacing.sm))
                    .h(key_list::list_height(total))
                    .w_full()
                    .child(
                        uniform_list(
                            "secret-keys-list",
                            total,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                let tokens = cx.tokens();
                                let Some(detail) = this.detail.upgrade() else {
                                    return Vec::new();
                                };
                                range
                                    .filter_map(|ix| {
                                        let key = rows.keys.get(ix)?;
                                        let cell = {
                                            let detail = detail.read(cx);
                                            match detail.revealed_line(&key.key) {
                                                Some(line) => Cell::Shown(line.clone()),
                                                None if detail.key_requested(&key.key) => {
                                                    Cell::Reading
                                                }
                                                None => Cell::Masked,
                                            }
                                        };
                                        let name = key.key.clone();
                                        let on_toggle = cx.listener(move |this, _, _, cx| {
                                            this.toggle_key(&name, cx)
                                        });
                                        Some(key_row(&tokens, ix, key, cell, on_toggle))
                                    })
                                    .collect()
                            }),
                        )
                        .size_full(),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }
}
