//! [`HelmBanner`]: the guidance banner of the Helm views (E16-S11).
//!
//! A banner, not a modal: it sits at the top of the page, says what is hidden and why, and how
//! to fix it. "Locate helm" runs `helm::LocateBinary` (opens the `helm.path` setting) and
//! "Recheck" runs `helm::Recheck` (detects the binary again); both are bus commands, so the
//! palette and agents reach them too. With a full gate it draws nothing, unless
//! `helm.server_side_apply` is on and the binary is too old for it: then a one-line warning.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, px,
};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{HelmFeature, HelmGate};
use oxikube_settings::helm::server_side_warning;
use oxikube_settings::{HelmSettings, Settings as _};
use oxikube_ui::button::Button;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use oxikube_workspace::CommandDispatcher;

use crate::gate::HelmGateState;

/// `build()` when `gate` allows `feature`, `None` otherwise: how a view hides an action or a
/// section (`.children(gated(&gate, HelmFeature::Install, || install_button))`).
pub fn gated<E>(gate: &HelmGate, feature: HelmFeature, build: impl FnOnce() -> E) -> Option<E> {
    gate.allows(feature).then(build)
}

/// The banner. See the [module docs](self).
pub struct HelmBanner {
    state: Entity<HelmGateState>,
    dispatcher: Rc<dyn CommandDispatcher>,
    _follow: [Subscription; 2],
}

impl HelmBanner {
    /// A banner over `state` whose buttons dispatch through `dispatcher`. It redraws when the
    /// gate changes and when `helm.server_side_apply` is edited.
    pub fn new(
        state: Entity<HelmGateState>,
        dispatcher: Rc<dyn CommandDispatcher>,
        cx: &mut Context<Self>,
    ) -> Self {
        let on_gate = cx.observe(&state, |_, _, cx| cx.notify());
        let on_settings = HelmSettings::observe_in(cx, |_, cx| cx.notify());
        Self {
            state,
            dispatcher,
            _follow: [on_gate, on_settings],
        }
    }

    /// A button that sends `command`, findable by tests as `id`.
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        command: Command,
        enabled: bool,
    ) -> AnyElement {
        let dispatcher = self.dispatcher.clone();
        div()
            .debug_selector(move || id.into())
            .child(
                Button::new(id)
                    .xsmall()
                    .label(label)
                    .disabled(!enabled)
                    .on_click(move |_, _, cx| dispatcher.dispatch(command.clone(), cx)),
            )
            .into_any_element()
    }
}

impl Render for HelmBanner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let state = self.state.read(cx);
        let checking = state.is_checking();
        let warning = server_side_warning(&HelmSettings::resolve(cx), &state.capabilities());
        let (text, error) = match (state.gate().guidance(), warning) {
            (Some(guidance), _) => (guidance, true),
            (None, Some(warning)) => (warning, false),
            (None, None) => return div().id("helm-banner-empty").into_any_element(),
        };
        let mutating_hidden = !state.gate().allows(HelmFeature::Install);
        let actions: AnyElement = h_flex()
            .gap(u(tokens.spacing.sm))
            .when(mutating_hidden, |row| {
                row.child(self.button(
                    "helm-locate",
                    "Locate helm",
                    Command::HelmLocateBinary,
                    true,
                ))
            })
            .child(self.button(
                "helm-recheck",
                if checking { "Checking..." } else { "Recheck" },
                Command::HelmRecheck,
                !checking,
            ))
            .into_any_element();
        v_flex()
            .id("helm-banner")
            .debug_selector(|| "helm-banner".into())
            .w_full()
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(if error { colors.warning } else { colors.border })
            .bg(colors.element)
            .child(
                div()
                    .id("helm-banner-text")
                    .text_size(u(px(13.)))
                    .text_color(colors.text)
                    .child(text),
            )
            .child(actions)
            .into_any_element()
    }
}
