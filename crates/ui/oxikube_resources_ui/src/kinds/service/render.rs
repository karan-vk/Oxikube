//! Drawing the Service panel from its prebuilt [`ServiceView`]: nothing is computed from the
//! cluster here. Every list is capped by the model, so a frame draws a bounded number of rows.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_domain::Capabilities;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, Tokens, u};

use super::browser::ServiceBrowser;
use super::model::{Pods, PortRow, ServiceView};
use super::panel::ServicePanel;
use crate::kinds::parts::{cell, note, section};
use crate::table::ToneColors;

/// The tallest the panel grows before it scrolls inside itself, unscaled pixels: the generic
/// Overview below it keeps the rest of the drawer.
const MAX_HEIGHT: f32 = 480.;
/// The width of a fact's label.
const LABEL_WIDTH: f32 = 120.;

impl ServicePanel {
    /// Whether the per-port buttons are drawn: a port-forward manager is installed and the
    /// session can forward.
    fn can_open_in_browser(&self, cx: &gpui::App) -> bool {
        if !ServiceBrowser::is_enabled(cx) {
            return false;
        }
        let (Some(detail), Some(target)) = (self.detail.upgrade(), &self.target) else {
            return false;
        };
        detail
            .read(cx)
            .deps()
            .sessions
            .get(&target.cluster)
            .is_some_and(|s| s.capabilities().contains(Capabilities::PORTFORWARD))
    }
}

impl ServicePanel {
    /// What the Forward buttons do: `None` when forwarding is not wired, else why they are
    /// disabled now, if they are.
    fn forward_state(&self, cx: &gpui::App) -> Option<Option<String>> {
        self.detail.upgrade()?.read(cx).forward_state()
    }
}

impl Render for ServicePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let tones = ToneColors::current(cx);
        let browser = self.can_open_in_browser(cx);
        let forward = self.forward_state(cx);
        let root = v_flex()
            .id("service-panel")
            .debug_selector(|| "service-panel".to_owned())
            .w_full()
            .max_h(u(px(MAX_HEIGHT)))
            .overflow_y_scroll()
            .border_b_1()
            .border_color(colors.border_variant)
            .pb(u(tokens.spacing.md));
        let Some(view) = self.view.as_ref() else {
            return root.child(note(&tokens, "Reading the service…"));
        };
        root.child(self.header(view, &tokens, cx))
            .children(view.health.as_ref().map(|(text, tone)| {
                div()
                    .debug_selector(|| "service-health".to_owned())
                    .px(u(tokens.spacing.lg))
                    .pb(u(tokens.spacing.sm))
                    .text_size(u(tokens.font.small))
                    .text_color(tones.of(*tone))
                    .child(text.clone())
            }))
            .child(facts(view, &tokens))
            .child(self.ports(view, browser, forward, &tokens, cx))
            .child(self.slices(view, &tokens, &tones, cx))
            .child(self.pods(view, &tokens, &tones, cx))
    }
}

impl ServicePanel {
    fn header(&self, view: &ServiceView, tokens: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let colors = tokens.colors;
        let selector = h_flex().gap(u(tokens.spacing.sm)).flex_wrap().children(
            view.selector.iter().enumerate().map(|(i, pair)| {
                div()
                    .debug_selector(move || format!("service-selector-{i}"))
                    .px(u(tokens.spacing.md))
                    .rounded(u(tokens.radius.sm))
                    .bg(colors.element)
                    .text_size(u(tokens.font.small))
                    .child(pair.clone())
            }),
        );
        let selects = !view.selector.is_empty() && !matches!(view.pods, Pods::None(_));
        h_flex()
            .items_center()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.lg))
            .pb(u(tokens.spacing.sm))
            .child(heading(tokens, "Service"))
            .child(div().flex_1().min_w_0().child(selector))
            .when(selects, |row| {
                row.child(
                    div()
                        .debug_selector(|| "service-show-pods".to_owned())
                        .child(
                            Button::new("service-show-pods")
                                .xsmall()
                                .ghost()
                                .icon(Icon::new(IconName::ExternalLink).size(u(px(13.))))
                                .label("Show pods")
                                .tooltip("Open the Pods table filtered by this selector")
                                .on_click(cx.listener(|this, _, _, cx| this.show_pods(cx))),
                        ),
                )
            })
            .into_any_element()
    }

    fn ports(
        &self,
        view: &ServiceView,
        browser: bool,
        forward: Option<Option<String>>,
        tokens: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if view.ports.is_empty() {
            return section(tokens, "Ports", 0)
                .child(note(tokens, "This service exposes no ports."))
                .into_any_element();
        }
        section(tokens, "Ports", view.ports.len())
            .children(
                view.ports
                    .iter()
                    .enumerate()
                    .map(|(i, port)| self.port_row(port, i, browser, forward.as_ref(), tokens, cx)),
            )
            .into_any_element()
    }

    fn port_row(
        &self,
        port: &PortRow,
        index: usize,
        browser: bool,
        forward: Option<&Option<String>>,
        tokens: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let number = port.port;
        let colors = tokens.colors;
        h_flex()
            .debug_selector(move || format!("service-port-{index}"))
            .items_center()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.lg))
            .py(u(px(1.)))
            .child(cell(port.label.clone(), 120., colors.text))
            .child(cell(port.text.clone(), 110., colors.text_muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(colors.text_muted)
                    .child(format!("→ {}", port.target)),
            )
            .when_some(forward.filter(|_| port.browsable), |row, blocked| {
                let tooltip = match blocked {
                    Some(reason) => reason.clone(),
                    None => format!("Forward port {number} to a local port"),
                };
                row.child(
                    div()
                        .debug_selector(move || format!("service-forward-{number}"))
                        .child(
                            Button::new(("service-forward", usize::from(number)))
                                .xsmall()
                                .ghost()
                                .icon(Icon::new(IconName::ExternalLink).size(u(px(13.))))
                                .label("Forward")
                                .disabled(blocked.is_some())
                                .tooltip(tooltip)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.forward(number, window, cx)
                                })),
                        ),
                )
            })
            .when(browser && port.browsable, |row| {
                row.child(
                    div()
                        .debug_selector(move || format!("service-open-{number}"))
                        .child(
                            Button::new(("service-open", usize::from(number)))
                                .xsmall()
                                .ghost()
                                .icon(Icon::new(IconName::Globe).size(u(px(13.))))
                                .label("Open in browser")
                                .tooltip(format!(
                                    "Forward port {number} to a pod and open it in the browser"
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_in_browser(number, cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

fn facts(view: &ServiceView, tokens: &Tokens) -> AnyElement {
    let colors = tokens.colors;
    v_flex()
        .px(u(tokens.spacing.lg))
        .pb(u(tokens.spacing.sm))
        .children(view.facts.iter().enumerate().map(|(i, (label, value))| {
            h_flex()
                .debug_selector(move || format!("service-fact-{i}"))
                .gap(u(tokens.spacing.md))
                .child(cell((*label).to_owned(), LABEL_WIDTH, colors.text_muted))
                .child(div().flex_1().min_w_0().truncate().child(value.clone()))
        }))
        .into_any_element()
}

fn heading(tokens: &Tokens, text: &'static str) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(u(tokens.font.small))
        .font_semibold()
        .text_color(tokens.colors.text_muted)
        .child(text.to_uppercase())
}
