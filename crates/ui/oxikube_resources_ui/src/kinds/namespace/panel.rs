//! The Namespace panel above the detail's Overview: the phase, what holds a terminating
//! namespace (finalizers and the controller's blockers), HNC's markers, and "Use namespace".

use gpui::{
    AnyView, App, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, WeakEntity, Window, div, px,
};
use oxikube_domain::view::{NamespacePhase, NamespaceSummary};
use oxikube_ui::button::Button;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use crate::detail::DetailView;

/// Builds the panel for one detail view (a [`DetailRenderer`](crate::kinds::DetailRenderer)
/// factory).
pub(super) fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        // Follow the object: a namespace that starts terminating redraws the panel.
        if let Some(view) = detail.upgrade() {
            cx.observe(&view, |_, _, cx| cx.notify()).detach();
        }
        NamespacePanel { detail }
    })
    .into()
}

/// The panel. See the [module docs](self).
pub struct NamespacePanel {
    detail: WeakEntity<DetailView>,
}

impl NamespacePanel {
    /// The namespace the detail shows, read off the object (one small JSON walk per draw).
    fn summary(&self, cx: &App) -> Option<NamespaceSummary> {
        let detail = self.detail.upgrade()?;
        let detail = detail.read(cx);
        NamespaceSummary::from_resource(detail.resource()?).ok()
    }

    /// Sends `namespace::Select` with this one namespace: "use this namespace" (k9s `u`).
    fn use_namespace(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (target, dispatcher) = {
            let detail = detail.read(cx);
            (detail.target().clone(), detail.dispatcher())
        };
        dispatcher.dispatch(super::actions::use_command(&target), cx);
    }
}

impl Render for NamespacePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(ns) = self.summary(cx) else {
            return div().into_any_element();
        };
        let phase_colour = match ns.phase {
            NamespacePhase::Active => colors.success,
            NamespacePhase::Terminating => colors.warning,
            NamespacePhase::Unknown => colors.text_muted,
        };
        let line = |id: &str, text: String| {
            let id = id.to_owned();
            div()
                .debug_selector(move || id)
                .w_full()
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(text)
        };
        let mut lines = Vec::new();
        if ns.is_terminating() {
            let mut text = String::from("Terminating");
            if let Some(since) = ns.deletion {
                text.push_str(&format!(" since {since}"));
            }
            text.push_str(": its contents are being removed.");
            lines.push(line("namespace-terminating", text).text_color(colors.warning));
            let finalizers: Vec<&str> = ns.all_finalizers().map(|f| f.as_ref()).collect();
            if !finalizers.is_empty() {
                lines.push(line(
                    "namespace-finalizers",
                    format!("Waiting for finalizers: {}", finalizers.join(", ")),
                ));
            }
            for (ix, blocker) in ns.blockers().enumerate() {
                let reason = blocker.message.as_deref().or(blocker.reason.as_deref());
                let text = match reason {
                    Some(reason) => format!("{}: {reason}", blocker.kind),
                    None => blocker.kind.to_string(),
                };
                lines.push(line(&format!("namespace-blocker-{ix}"), text));
            }
        }
        if let Some(parent) = &ns.hnc.subnamespace_of {
            lines.push(line(
                "namespace-hnc",
                format!("HNC subnamespace of {parent}"),
            ));
        } else if ns.hnc.managed {
            lines.push(line("namespace-hnc", "Managed by HNC".to_owned()));
        }
        if let Some(reason) = ns.protected() {
            lines.push(line("namespace-protected", reason.to_owned()).text_color(colors.warning));
        }
        v_flex()
            .debug_selector(|| "namespace-panel".to_owned())
            .w_full()
            .gap(u(px(4.)))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .items_center()
                    .gap(u(tokens.spacing.md))
                    .child(
                        div()
                            .debug_selector(|| "namespace-phase".to_owned())
                            .text_color(phase_colour)
                            .child(ns.phase.as_str()),
                    )
                    .child(div().flex_1())
                    .child(
                        div().debug_selector(|| "namespace-use".to_owned()).child(
                            Button::new("namespace-use")
                                .label("Use namespace")
                                .xsmall()
                                .tooltip("Watch only this namespace (u)")
                                .on_click(cx.listener(|this, _, _, cx| this.use_namespace(cx))),
                        ),
                    ),
            )
            .children(lines)
            .into_any_element()
    }
}
