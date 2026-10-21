//! The repository rows.

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Colors, Sizable as _, u};

use super::super::model::{RowModel, Tone};
use super::super::view::HelmReposView;

fn tone_colour(tone: Tone, colors: &Colors) -> Hsla {
    match tone {
        Tone::Neutral => colors.text,
        Tone::Good => colors.success,
        Tone::Pending => colors.info,
        Tone::Error => colors.error,
        Tone::Muted => colors.text_muted,
    }
}

impl HelmReposView {
    pub(super) fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        if self.model.rows.is_empty() {
            let text = if self.model.loaded {
                "No chart repositories yet. Add one above, or pick a suggestion."
            } else {
                "Reading Helm's repositories..."
            };
            return div()
                .debug_selector(|| "helm-repos-empty".into())
                .text_color(colors.text_muted)
                .child(text)
                .into_any_element();
        }
        // A machine has a handful of repositories, so the rows are plain children.
        let mut list = v_flex().debug_selector(|| "helm-repos-list".into());
        for (index, row) in self.model.rows.iter().enumerate() {
            list = list.child(self.row(index, row, cx));
        }
        list.into_any_element()
    }

    fn row(&self, index: usize, row: &RowModel, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let busy = self.model.busy;
        let confirming = self.confirm_remove.as_deref() == Some(row.name.as_str());
        let (update_name, remove_name) = (row.name.clone(), row.name.clone());
        let (row_sel, status_sel) = (
            format!("helm-repo-row:{}", row.name),
            format!("helm-repo-status:{}:{}", row.name, tone_name(row.tone)),
        );
        h_flex()
            .id(("helm-repo", index))
            .debug_selector(move || row_sel)
            .gap(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .items_start()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .w(u(px(180.)))
                    .flex_none()
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(SharedString::from(row.name.clone())),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(u(px(2.)))
                    .child(
                        div()
                            .truncate()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(row.url.clone())),
                    )
                    // The reason a refresh failed can be a sentence: it wraps instead of cutting.
                    .child(
                        div()
                            .debug_selector(move || status_sel)
                            .text_size(u(tokens.font.small))
                            .text_color(tone_colour(row.tone, &colors))
                            .child(SharedString::from(row.status.clone())),
                    ),
            )
            .child(
                div()
                    .debug_selector({
                        let name = row.name.clone();
                        move || format!("helm-repo-update:{name}")
                    })
                    .child(
                        Button::new(("helm-repo-update", index))
                            .xsmall()
                            .outline()
                            .label("Update")
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.update_one(&update_name, cx)
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .debug_selector({
                        let name = row.name.clone();
                        move || format!("helm-repo-remove:{name}")
                    })
                    .child(
                        Button::new(("helm-repo-remove", index))
                            .xsmall()
                            .ghost()
                            .label(if confirming {
                                "Click again to remove"
                            } else {
                                "Remove"
                            })
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove(&remove_name, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral => "neutral",
        Tone::Good => "good",
        Tone::Pending => "pending",
        Tone::Error => "error",
        Tone::Muted => "muted",
    }
}
