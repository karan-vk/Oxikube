//! The IngressClass panel: controller, parameters, the Default badge and "Set as default".

use gpui::prelude::*;
use gpui::{AnyElement, Context};
use oxikube_app::columns::Tone;
use oxikube_domain::command::Command;
use oxikube_domain::view::network::IngressClassSummary;
use oxikube_ui::button::Button;
use oxikube_ui::layout::Disableable as _;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Sizable as _, u};

use super::panel::NetworkPanel;
use super::widgets::{Look, chip, field, note, shell};

impl NetworkPanel {
    pub(super) fn ingress_class(
        &self,
        look: &Look,
        summary: &IngressClassSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let controller = summary
            .controller
            .as_deref()
            .unwrap_or("not set")
            .to_owned();
        let parameters = match &summary.parameters {
            None => "none".to_owned(),
            Some(p) => {
                let group = p
                    .api_group
                    .as_deref()
                    .map(|g| format!("{g}/"))
                    .unwrap_or_default();
                let place = match &p.namespace {
                    Some(ns) => format!("{ns}/{}", p.name),
                    None => p.name.clone(),
                };
                format!("{group}{} {place} ({} scope)", p.kind, p.scope)
            }
        };
        let status = if summary.is_default {
            chip(look, "net-class-default", "Default", Tone::Ok).into_any_element()
        } else {
            note(look, "net-class-not-default", "Not the default class").into_any_element()
        };
        let mut panel = shell(look, "net-ingress-class")
            .child(field(
                look,
                "net-class-controller",
                "Controller",
                controller,
            ))
            .child(field(
                look,
                "net-class-parameters",
                "Parameters",
                parameters,
            ))
            .child(field(look, "net-class-status", "Default", status));
        if !summary.is_default {
            panel = panel.child(self.set_default_row(look, cx));
        }
        panel.into_any_element()
    }

    /// The "Set as default" button and what it will do. The command is the guard's to confirm
    /// (low risk), to refuse on a read-only cluster and to audit; the button only says so.
    fn set_default_row(&self, look: &Look, cx: &mut Context<Self>) -> AnyElement {
        let read_only = self.read_only();
        let tooltip = if read_only {
            "This cluster is read-only"
        } else {
            "Make this the default Ingress class"
        };
        h_flex()
            .gap(u(look.tokens.spacing.md))
            .items_center()
            .child(
                gpui::div()
                    .debug_selector(|| "net-class-set-default".to_owned())
                    .child(
                        Button::new("net-class-set-default")
                            .xsmall()
                            .label("Set as default")
                            .tooltip(tooltip)
                            .disabled(read_only)
                            .on_click(cx.listener(|this, _, _, cx| this.set_default(cx))),
                    ),
            )
            .child(note(
                look,
                "net-class-set-default-note",
                "Asks first, and clears the flag on the class that is the default now.",
            ))
            .into_any_element()
    }

    /// Sends `ingressclass::SetDefault` for this class.
    fn set_default(&self, cx: &mut Context<Self>) {
        let Some(target) = self.object().cloned() else {
            return;
        };
        let command = Command::IngressClassSetDefault {
            target,
            clear_previous: true,
        };
        self.dispatch(command, cx);
    }
}
