//! The Overview tab: header-less body of one virtualised list. Metadata (owners, labels with
//! copy, annotations with expand and copy, finalizers), the conditions table, the `status`
//! summary, and for a Secret the key names. Long lists cost only the rows on screen.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, list, px,
};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Tokens, u};

use super::model::{DetailModel, MetaEntry, Row};
use super::parts::{
    KEY_WIDTH, condition_row, empty, full_width, muted, section_heading, skeleton, status_line,
    truncated,
};
use super::state::FullState;
use super::view::DetailView;
use crate::table::ToneColors;

impl DetailView {
    /// The Overview: the list, or a skeleton until the object is known.
    pub(super) fn overview_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.model.is_none() {
            let tokens = cx.tokens();
            return skeleton(&tokens, "detail-skeleton", &[260., 180., 320., 140., 220.]);
        }
        div()
            .debug_selector(|| "detail-overview".to_owned())
            .size_full()
            .child(
                list(
                    self.overview.clone(),
                    cx.processor(|this, ix, _, cx| full_width(this.overview_row(ix, cx))),
                )
                .size_full(),
            )
            .into_any_element()
    }

    /// One row of the Overview. Only rows on screen are built.
    fn overview_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let (Some(row), Some(model)) = (self.body.get(ix).copied(), self.model.as_ref()) else {
            return empty();
        };
        #[cfg(test)]
        {
            self.rendered_rows += 1;
        }
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let padded = |child: AnyElement| {
            div()
                .px(u(tokens.spacing.lg))
                .py(u(px(2.)))
                .child(child)
                .into_any_element()
        };
        match row {
            Row::Section(section, count) => section_heading(&tokens, section, count),
            Row::Empty(section) => padded(muted(&tokens, section.empty_text())),
            Row::Loading => match &self.full {
                FullState::Failed(message) => padded(muted(
                    &tokens,
                    &format!("The full object could not be read: {message}"),
                )),
                _ => skeleton(&tokens, "detail-loading", &[220., 160., 260.]),
            },
            Row::Label(i) => model
                .labels
                .get(i)
                .map(|entry| self.meta_row(&tokens, entry, i, false, cx))
                .unwrap_or_else(empty),
            Row::Annotation(i) => model
                .annotations
                .get(i)
                .map(|entry| self.meta_row(&tokens, entry, i, true, cx))
                .unwrap_or_else(empty),
            Row::Owner(i) => self.owner_row(&tokens, model, i, cx),
            Row::Finalizer(i) => padded(
                div()
                    .debug_selector(move || format!("detail-finalizer-{i}"))
                    .child(model.finalizers.get(i).cloned().unwrap_or_default())
                    .into_any_element(),
            ),
            Row::Condition(i) => model
                .conditions
                .get(i)
                .map(|row| condition_row(&tokens, &tones, row, i, self.now()))
                .unwrap_or_else(empty),
            Row::Status(i) => model
                .status
                .lines
                .get(i)
                .map(|line| status_line(&tokens, line, i))
                .unwrap_or_else(empty),
            Row::StatusTruncated => padded(muted(&tokens, "More fields are not shown.")),
            Row::SecretKey(i) => {
                let key = model
                    .secret_keys
                    .as_ref()
                    .and_then(|keys| keys.get(i))
                    .cloned()
                    .unwrap_or_default();
                padded(
                    h_flex()
                        .debug_selector(move || format!("detail-secret-key-{i}"))
                        .gap(u(tokens.spacing.md))
                        .child(div().child(key))
                        .child(muted(&tokens, "value hidden"))
                        .into_any_element(),
                )
            }
        }
    }

    /// A label or annotation: key, value (cut until expanded), copy.
    fn meta_row(
        &self,
        tokens: &Tokens,
        entry: &MetaEntry,
        index: usize,
        annotation: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let expanded = self.expanded.contains(&(annotation, entry.key.clone()));
        let shown = if expanded {
            entry.value.to_string()
        } else {
            entry.collapsed()
        };
        let (prefix, expand_id, copy_id, key_name) = if annotation {
            ("annotation", "expand-a", "copy-a", "detail-annotation-key")
        } else {
            ("label", "expand-l", "copy-l", "detail-label-key")
        };
        let (copy_key, expand_key) = (entry.key.clone(), entry.key.clone());
        let expandable = entry.expandable();
        let copyable = entry.copyable;
        h_flex()
            .id((prefix, index))
            .debug_selector(move || format!("detail-{prefix}-{index}"))
            .items_start()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .py(u(px(2.)))
            .child(
                truncated(key_name, index, entry.key.to_string())
                    .flex_none()
                    .w(u(px(KEY_WIDTH)))
                    .text_color(colors.text_muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_normal()
                    .child(shown),
            )
            .when(expandable, |row| {
                row.child(
                    div()
                        .id((expand_id, index))
                        .debug_selector(move || format!("detail-expand-{prefix}-{index}"))
                        .cursor_pointer()
                        .flex_none()
                        .text_color(colors.accent)
                        .text_size(u(tokens.font.small))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_expanded(annotation, &expand_key, cx);
                        }))
                        .child(if expanded { "Less" } else { "More" }),
                )
            })
            .when(copyable, |row| {
                row.child(
                    div()
                        .id((copy_id, index))
                        .debug_selector(move || format!("detail-copy-{prefix}-{index}"))
                        .cursor_pointer()
                        .flex_none()
                        .text_color(colors.text_muted)
                        .hover(|style| style.text_color(colors.text))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_meta(&copy_key, annotation, cx);
                        }))
                        .child(Icon::new(IconName::Copy).size(u(px(13.)))),
                )
            })
            .into_any_element()
    }

    /// An owner reference: a link that opens the owner's detail once its scope is known.
    fn owner_row(
        &self,
        tokens: &Tokens,
        model: &DetailModel,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = tokens.colors;
        let Some(owner) = model.owners.get(index) else {
            return empty();
        };
        let openable = self.owner_target(owner).is_some();
        let label = format!("{}/{}", owner.gvk.kind, owner.name);
        let link = div()
            .id(("owner", index))
            .debug_selector(move || format!("detail-owner-{index}"))
            .when(openable, |link| {
                link.cursor_pointer()
                    .text_color(colors.accent)
                    .hover(|style| style.underline())
                    .on_click(cx.listener(move |this, _, _, cx| this.open_owner(index, cx)))
            })
            .when(!openable, |link| link.text_color(colors.text_muted))
            .child(label);
        h_flex()
            .px(u(tokens.spacing.lg))
            .py(u(px(2.)))
            .gap(u(tokens.spacing.md))
            .child(link)
            .when(owner.controller, |row| {
                row.child(muted(tokens, "controller"))
            })
            .into_any_element()
    }

    /// Expands or collapses the value of `key`, and tells the list its rows changed height.
    pub fn toggle_expanded(&mut self, annotation: bool, key: &str, cx: &mut Context<Self>) {
        let entry = (annotation, Arc::<str>::from(key));
        if !self.expanded.remove(&entry) {
            self.expanded.insert(entry);
        }
        self.overview.remeasure_items(0..self.body.len());
        cx.notify();
    }

    /// Whether the value of `key` is expanded.
    pub fn is_expanded(&self, annotation: bool, key: &str) -> bool {
        self.expanded
            .iter()
            .any(|(a, k)| *a == annotation && &**k == key)
    }
}
