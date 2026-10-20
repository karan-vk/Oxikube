//! The Forward buttons under the containers table (E15-S04): one per TCP container port, opening
//! the port-forward start dialog on that port with the pod's ports to choose from.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px,
};
use oxikube_domain::portforward::ForwardPort;
use oxikube_portforward_ui::{ForwardRequest, PortChoice};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex};
use oxikube_ui::{Icon, IconName, Sizable as _, Tokens, u};

use super::ContainersPanel;

impl ContainersPanel {
    /// What the Forward buttons do: `None` when forwarding is not wired, else why they are
    /// disabled now, if they are.
    fn forward_state(&self, cx: &gpui::App) -> Option<Option<String>> {
        self.detail.upgrade()?.read(cx).forward_state()
    }

    /// The row of Forward buttons, or nothing without ports or without a forward flow.
    pub(super) fn forward_row(
        &self,
        tokens: &Tokens,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.ports.is_empty() {
            return None;
        }
        let blocked = self.forward_state(cx)?;
        let colors = tokens.colors;
        Some(
            h_flex()
                .debug_selector(|| "pod-forward".to_owned())
                .items_center()
                .flex_wrap()
                .gap(u(tokens.spacing.md))
                .px(u(tokens.spacing.lg))
                .pt(u(tokens.spacing.sm))
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(colors.text_muted)
                        .child("Forward a port"),
                )
                .children(self.ports.iter().enumerate().map(|(i, port)| {
                    let tooltip = match &blocked {
                        Some(reason) => reason.clone(),
                        None => format!("Forward {} to a local port", port.label()),
                    };
                    let number = port.number;
                    div()
                        .debug_selector(move || format!("pod-forward-{number}"))
                        .child(
                            Button::new(("pod-forward", i))
                                .xsmall()
                                .ghost()
                                .icon(Icon::new(IconName::ExternalLink).size(u(px(13.))))
                                .label(format!("Forward {}", port.label()))
                                .disabled(blocked.is_some())
                                .tooltip(tooltip)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.forward(number, window, cx)
                                })),
                        )
                }))
                .into_any_element(),
        )
    }

    /// Opens the start dialog on `port`, with the pod's ports to choose from.
    pub fn forward(&mut self, port: u16, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let target = detail.read(cx).target().clone();
        let ports: Vec<PortChoice> = self.ports.clone();
        let request = ForwardRequest::new(target)
            .with_port(ForwardPort::Number(port))
            .with_ports(ports);
        detail.update(cx, |detail, cx| detail.open_forward(request, window, cx));
    }

    /// The TCP container ports the Forward buttons are for.
    pub fn ports(&self) -> &[PortChoice] {
        &self.ports
    }
}
