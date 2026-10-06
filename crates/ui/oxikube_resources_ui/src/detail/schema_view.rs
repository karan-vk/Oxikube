//! Drawing the Schema tab (E07-S07): the summary, the version chips, the way to the custom
//! resources and the virtualised tree. Everything drawn comes from the rows [`schema_tab`]
//! keeps; nothing is computed here.
//!
//! [`schema_tab`]: super::schema_tab

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, list, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

use super::parts::{empty, full_width, skeleton};
use super::view::DetailView;
use crate::crds::{CrdInfo, RowKind};

impl DetailView {
    /// The Schema tab: a skeleton until the CRD is known, a note when it declares no schema,
    /// else the summary, the version chips and the tree.
    pub(super) fn schema_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let Some(info) = self.schema.info.as_ref() else {
            return skeleton(&tokens, "detail-schema-loading", &[260., 200., 300.]);
        };
        let note = |selector: &'static str, text: &str| {
            div()
                .debug_selector(move || selector.to_owned())
                .p(u(tokens.spacing.xl))
                .text_color(tokens.colors.text_muted)
                .child(text.to_owned())
                .into_any_element()
        };
        let body = if self.schema.versions.is_empty() {
            note(
                "detail-schema-none",
                "This CRD declares no schema: it accepts any fields.",
            )
        } else if self.schema.rows.rows.is_empty() {
            note(
                "detail-schema-empty",
                "This version's schema lists no fields (it accepts any).",
            )
        } else {
            div()
                .debug_selector(|| "detail-schema-tree".to_owned())
                .size_full()
                .child(
                    list(
                        self.schema.list.clone(),
                        cx.processor(|this, ix, _, cx| full_width(this.schema_row(ix, cx))),
                    )
                    .size_full(),
                )
                .into_any_element()
        };
        v_flex()
            .size_full()
            .child(self.schema_header(info, &tokens, cx))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// Kind, group, scope, names, the version chips and the way to the custom resources.
    fn schema_header(&self, info: &CrdInfo, tokens: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let colors = tokens.colors;
        let mut facts = vec![
            info.kind.clone(),
            info.group.clone(),
            format!("{:?}", info.scope),
        ];
        if !info.short_names.is_empty() {
            facts.push(format!("short names: {}", info.short_names.join(", ")));
        }
        if !info.categories.is_empty() {
            facts.push(format!("categories: {}", info.categories.join(", ")));
        }
        let chips = self.schema.versions.iter().map(|name| {
            let active = self.schema.version.as_deref() == Some(name.as_str());
            let flags = info
                .versions
                .iter()
                .find(|v| &v.name == name)
                .map(|v| {
                    [
                        v.storage.then_some("storage"),
                        v.deprecated.then_some("deprecated"),
                        (!v.served).then_some("not served"),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(", ")
                })
                .unwrap_or_default();
            let label = if flags.is_empty() {
                name.clone()
            } else {
                format!("{name} ({flags})")
            };
            let target = name.clone();
            div()
                .id(("schema-version", self.schema_chip_id(name)))
                .debug_selector({
                    let name = name.clone();
                    move || format!("detail-schema-version-{name}")
                })
                .cursor_pointer()
                .px(u(tokens.spacing.md))
                .rounded(u(tokens.radius.sm))
                .text_size(u(tokens.font.small))
                .bg(if active {
                    colors.element_selected
                } else {
                    colors.element
                })
                .text_color(if active {
                    colors.text
                } else {
                    colors.text_muted
                })
                .hover(|style| style.text_color(colors.text))
                .on_click(cx.listener(move |this, _, _, cx| this.set_schema_version(&target, cx)))
                .child(label)
        });
        v_flex()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "detail-schema-facts".to_owned())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(facts.join(" · ")),
                    )
                    .child(
                        div()
                            .debug_selector(|| "detail-open-resources".to_owned())
                            .child(
                                Button::new("detail-open-resources")
                                    .xsmall()
                                    .ghost()
                                    .icon(Icon::new(IconName::ExternalLink).size(u(px(13.))))
                                    .label("Open custom resources")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.open_custom_resources(cx)
                                        }),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .flex_wrap()
                    .children(chips),
            )
            .into_any_element()
    }

    /// A stable small number for a version chip's element id.
    fn schema_chip_id(&self, name: &str) -> usize {
        self.schema
            .versions
            .iter()
            .position(|v| v == name)
            .unwrap_or_default()
    }

    /// One row of the tree. Only rows on screen are built.
    fn schema_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.schema.rows.rows.get(ix) else {
            return empty();
        };
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let key: Arc<str> = row.key.clone();
        let indent = f32::from(u8::try_from(row.depth).unwrap_or(u8::MAX)) * 16.;
        if row.kind != RowKind::Field {
            return div()
                .debug_selector(move || format!("schema-row-{key}"))
                .pl(u(px(8. + indent)))
                .py(u(px(2.)))
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(row.name.clone())
                .into_any_element();
        }
        let chevron = div()
            .w(u(px(14.)))
            .flex_none()
            .when(row.expandable, |slot| {
                slot.child(
                    Icon::new(if row.open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(u(px(14.)))
                    .color(colors.text_muted),
                )
            });
        let toggle_key = key.clone();
        let mut details = Vec::new();
        if let Some(description) = &row.description {
            details.push(description.clone());
        }
        if !row.enum_values.is_empty() {
            details.push(format!("enum: {}", row.enum_values.join(" | ")));
        }
        if let Some(default) = &row.default {
            details.push(format!("default: {default}"));
        }
        v_flex()
            .id(("schema-row", ix))
            .debug_selector(move || format!("schema-row-{key}"))
            .pl(u(px(8. + indent)))
            .pr(u(tokens.spacing.lg))
            .py(u(px(2.)))
            .when(row.expandable, |row| {
                row.cursor_pointer()
                    .hover(|style| style.bg(colors.element_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_schema(&toggle_key, cx);
                    }))
            })
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .items_center()
                    .child(chevron)
                    .child(div().font_semibold().child(row.name.clone()))
                    .when(row.required, |line| {
                        line.child(
                            div()
                                .text_size(u(tokens.font.small))
                                .text_color(colors.warning)
                                .child("required"),
                        )
                    })
                    .child(
                        div()
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(row.ty.clone()),
                    ),
            )
            .children(details.into_iter().map(|line| {
                div()
                    .pl(u(px(20.)))
                    .whitespace_normal()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(line)
            }))
            .into_any_element()
    }
}
