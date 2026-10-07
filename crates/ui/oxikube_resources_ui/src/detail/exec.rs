//! "Shell" and "Attach" in a pod's header (E09-S08): the same flow as the table's row actions
//! ([`ExecFlow`](crate::exec::ExecFlow)), so a pod with several containers asks first and the
//! command carries the container that opens.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px,
};
use oxikube_app::ActionContext;
use oxikube_domain::Capabilities;
use oxikube_domain::command::{self, CommandId};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::Disableable as _;
use oxikube_ui::{Icon, IconName, Sizable as _, u};

use super::view::DetailView;
use crate::exec::ExecKind;

impl DetailView {
    /// What the header does about shells: `None` when it offers none (not a pod, no flow wired,
    /// the session may not exec), else why they are blocked now, if they are (a read-only cluster
    /// that does not allow them).
    fn exec_state(&self) -> Option<Option<String>> {
        let is_pod = self.target.gvk.group.is_empty() && &*self.target.gvk.kind == "Pod";
        if !is_pod || self.deps.exec.is_none() {
            return None;
        }
        let session = self.deps.sessions.get(&self.target.cluster)?;
        if !session.capabilities().contains(Capabilities::EXEC) {
            return None;
        }
        let meta = command::lookup(CommandId::POD_SHELL)?;
        Some(
            ActionContext::of(&session)
                .state_of(meta)
                .reason()
                .map(|reason| reason.to_string()),
        )
    }

    /// Starts a session of `kind` in this pod: reads the pod, then asks which container (several)
    /// or dispatches the command. A read-only cluster that blocks shells says why on the button
    /// instead (it is disabled), so nothing happens here.
    pub fn open_session(&mut self, kind: ExecKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.exec_state() != Some(None) {
            return;
        }
        let Some(flow) = self.deps.exec.clone() else {
            return;
        };
        self.exec_task = Some(flow.begin(kind, self.target.clone(), window, cx));
    }

    /// The header's two buttons, for a pod the session may exec into; `None` for anything else.
    pub(super) fn exec_buttons(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let blocked = self.exec_state()?;
        let button = |kind: ExecKind, id: &'static str, icon: IconName, tip: &'static str| {
            let tooltip = blocked.clone().unwrap_or_else(|| tip.to_owned());
            div().debug_selector(move || id.to_owned()).child(
                Button::new(id)
                    .xsmall()
                    .ghost()
                    .icon(Icon::new(icon).size(u(px(14.))))
                    .tooltip(tooltip)
                    .disabled(blocked.is_some())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_session(kind, window, cx)),
                    ),
            )
        };
        Some(
            div()
                .flex()
                .gap(u(px(2.)))
                .child(button(
                    ExecKind::Shell,
                    "detail-shell",
                    IconName::Terminal,
                    "Open a shell",
                ))
                .child(button(
                    ExecKind::Attach,
                    "detail-attach",
                    IconName::Container,
                    "Attach to the main process",
                ))
                .into_any_element(),
        )
    }
}
