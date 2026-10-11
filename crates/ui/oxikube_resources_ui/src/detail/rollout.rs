//! "Restart" and "Set image" in the header of a Deployment, StatefulSet or DaemonSet's detail
//! (E12-S05): the same actions as the table's menu and keys. Restart sends `workload::Restart`
//! through the bus (which asks its one confirmation and answers with a toast); Set image opens the
//! [`SetImageDialog`](crate::kinds::SetImageDialog) through the same [`SetImageFlow`] as the
//! table. The palette runs both from a focused detail through here.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px,
};
use oxikube_app::ActionContext;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::Disableable as _;
use oxikube_ui::{Icon, IconName, Sizable as _, u};

use super::view::DetailView;

impl DetailView {
    /// What the view does about the rollout verbs: `None` when it offers none (no flow wired, a
    /// kind a template change does not roll, no session), else why they are blocked now, if they
    /// are (a read-only cluster). Both verbs mutate, so they share the answer.
    pub fn rollout_state(&self) -> Option<Option<String>> {
        self.deps.set_image.as_ref()?;
        if !oxikube_app::is_rollable(&self.target.gvk) {
            return None;
        }
        let session = self.deps.sessions.get(&self.target.cluster)?;
        let meta = command::lookup(CommandId::WORKLOAD_RESTART)?;
        Some(
            ActionContext::of(&session)
                .state_of(meta)
                .reason()
                .map(|reason| reason.to_string()),
        )
    }

    /// Restarts the workload: sends `workload::Restart`, which the bus confirms and guards. Does
    /// nothing when the verbs are not on offer or are blocked (the button says why).
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.rollout_state() != Some(None) {
            return;
        }
        let command = Command::WorkloadRestart {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Opens the set-image dialog for the workload. Does nothing when the verbs are not on offer
    /// or are blocked (the button says why).
    pub fn open_set_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rollout_state() != Some(None) {
            return;
        }
        if let Some(flow) = self.deps.set_image.clone() {
            flow.begin(self.target.clone(), window, cx);
        }
    }

    /// The header's two buttons; `None` for a kind that has none.
    pub(super) fn rollout_buttons(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let blocked = self.rollout_state()?;
        let button = |id: &'static str, icon: IconName, tip: &'static str| {
            let tooltip = blocked.clone().unwrap_or_else(|| tip.to_owned());
            (
                div().debug_selector(move || id.to_owned()),
                Button::new(id)
                    .xsmall()
                    .ghost()
                    .icon(Icon::new(icon).size(u(px(14.))))
                    .tooltip(tooltip)
                    .disabled(blocked.is_some()),
            )
        };
        let (restart_slot, restart) = button(
            "detail-restart",
            IconName::RefreshCw,
            "Restart: roll new pods (r)",
        );
        let (image_slot, image) = button(
            "detail-set-image",
            IconName::Tag,
            "Set the images of the containers (i)",
        );
        Some(
            div()
                .flex()
                .gap(u(px(2.)))
                .child(
                    restart_slot
                        .child(restart.on_click(cx.listener(|this, _, _, cx| this.restart(cx)))),
                )
                .child(image_slot.child(
                    image.on_click(
                        cx.listener(|this, _, window, cx| this.open_set_image(window, cx)),
                    ),
                ))
                .into_any_element(),
        )
    }
}
