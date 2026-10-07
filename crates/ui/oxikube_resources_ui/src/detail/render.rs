//! Drawing the detail view: header, banner, tab strip and the body of the active tab.
//!
//! Everything drawn comes from the model and the view's own state; nothing is computed from the
//! cluster here, and the long bodies (Overview, Events) are virtualised lists.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::state::{DetailState, Mount};
use super::tabs::DetailTab;
use super::view::DetailView;
use crate::table::ToneColors;

impl Render for DetailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let deleted = matches!(self.state, DetailState::Deleted);
        let body = match self.tab {
            DetailTab::Overview => self.overview_body(cx),
            DetailTab::Events => self.events_body(cx),
            DetailTab::Schema => self.schema_body(cx),
            DetailTab::Yaml => self.yaml_body(window, cx),
            DetailTab::Describe => self.describe_body(window, cx),
        };
        v_flex()
            .id("detail-view")
            .debug_selector(|| "detail-view".to_owned())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_close_action))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_show_tab))
            .size_full()
            .overflow_hidden()
            .bg(colors.surface)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(self.header(cx))
            .children(self.banner(cx))
            .child(self.tab_strip(cx))
            .child(
                div()
                    .id("detail-body")
                    .flex_1()
                    .min_h_0()
                    .when(deleted, |body| body.opacity(0.55))
                    .child(body),
            )
    }
}

impl DetailView {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let tones = ToneColors::current(cx);
        let now = self.now();
        let (kind, name) = (self.target.gvk.kind.clone(), self.target.name.clone());
        let header = self.model.as_ref().map(|m| &m.header);
        let namespace = self.target.namespace.clone();
        let age = header.and_then(|h| h.age(now));
        let chip = header.and_then(|h| h.status.clone());
        let mount = self.mount;
        let meta_line = h_flex()
            .gap(u(tokens.spacing.md))
            .items_center()
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .children(namespace.map(|ns| {
                div()
                    .debug_selector(|| "detail-namespace".to_owned())
                    .child(format!("namespace {ns}"))
            }))
            .children(age.map(|age| {
                div()
                    .debug_selector(|| "detail-age".to_owned())
                    .child(format!("age {age}"))
            }))
            .children(chip.map(|chip| {
                let color = tones.of(chip.tone);
                div()
                    .debug_selector(|| "detail-status".to_owned())
                    .px(u(tokens.spacing.md))
                    .rounded(u(tokens.radius.sm))
                    .bg(color.opacity(0.14))
                    .text_color(color)
                    .child(chip.text)
            }));
        v_flex()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.lg))
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "detail-kind".to_owned())
                            .flex_none()
                            .px(u(tokens.spacing.md))
                            .rounded(u(tokens.radius.sm))
                            .bg(colors.element)
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(kind.to_string()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "detail-name".to_owned())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(u(tokens.font.heading))
                            .font_semibold()
                            .child(name.to_string()),
                    )
                    .children(self.exec_buttons(cx))
                    .when(mount == Mount::Drawer, |row| {
                        row.child(
                            div().debug_selector(|| "detail-pin".to_owned()).child(
                                Button::new("detail-pin")
                                    .xsmall()
                                    .ghost()
                                    .icon(Icon::new(IconName::Bookmark).size(u(px(14.))))
                                    .tooltip("Pin as tab")
                                    .on_click(cx.listener(|this, _, _, cx| this.request_pin(cx))),
                            ),
                        )
                        .child(
                            div().debug_selector(|| "detail-close".to_owned()).child(
                                Button::new("detail-close")
                                    .xsmall()
                                    .ghost()
                                    .icon(Icon::new(IconName::X).size(u(px(14.))))
                                    .tooltip("Close")
                                    .on_click(cx.listener(|this, _, _, cx| this.request_close(cx))),
                            ),
                        )
                    }),
            )
            .child(meta_line)
    }

    /// The line under the header for an object that is gone or cannot be read.
    fn banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let tones = ToneColors::current(cx);
        let (text, color) = match &self.state {
            DetailState::Deleted => (
                "This object was deleted. The last known state is shown.".to_owned(),
                tones.warn,
            ),
            DetailState::NotFound => (
                "Not found: it may have been deleted, or is outside the namespaces selected."
                    .to_owned(),
                tones.warn,
            ),
            DetailState::Unavailable(message) => (message.clone(), tones.error),
            DetailState::Loading | DetailState::Live => return None,
        };
        Some(
            div()
                .debug_selector(|| "detail-banner".to_owned())
                .flex_none()
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.md))
                .bg(color.opacity(0.14))
                .text_color(color)
                .text_size(u(tokens.font.small))
                .child(text)
                .into_any_element(),
        )
    }

    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        h_flex()
            .flex_none()
            .px(u(tokens.spacing.md))
            .gap(u(tokens.spacing.sm))
            .border_b_1()
            .border_color(colors.border_variant)
            .children(DetailTab::for_kind(&self.target.gvk).iter().map(|&tab| {
                let active = tab == self.tab;
                div()
                    .id(("detail-tab", tab as usize))
                    .debug_selector(move || format!("detail-tab-{}", tab.id()))
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.md))
                    .cursor_pointer()
                    .text_size(u(tokens.font.body))
                    .border_b_2()
                    .border_color(if active {
                        colors.accent
                    } else {
                        gpui::transparent_black()
                    })
                    .text_color(if active {
                        colors.text
                    } else {
                        colors.text_muted
                    })
                    .hover(|style| style.text_color(colors.text))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
                    .child(tab.title())
            }))
    }
}
