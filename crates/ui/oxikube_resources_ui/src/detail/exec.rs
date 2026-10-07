//! "Shell", "Attach" and "Debug" in a pod's header (E09-S08, E09-S10): the same flow as the table's row actions
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
use crate::exec::{ExecFlow, ExecKind};

impl DetailView {
    /// What the header does about `command`: `None` when it offers none (not a pod, the session
    /// may not exec), else why it is blocked now, if it is (a read-only cluster).
    fn command_state(&self, command: CommandId) -> Option<Option<String>> {
        if !self.target.gvk.is_pod() {
            return None;
        }
        let session = self.deps.sessions.get(&self.target.cluster)?;
        if !session.capabilities().contains(Capabilities::EXEC) {
            return None;
        }
        let meta = command::lookup(command)?;
        Some(
            ActionContext::of(&session)
                .state_of(meta)
                .reason()
                .map(|reason| reason.to_string()),
        )
    }

    /// What the header does about shells: `None` when it offers none (no flow wired, see
    /// [`command_state`](Self::command_state)).
    fn exec_state(&self) -> Option<Option<String>> {
        self.deps.exec.as_ref()?;
        self.command_state(CommandId::POD_SHELL)
    }

    /// What the header does about debug containers: `None` when it offers none (no debug flow
    /// wired), else why it is blocked now (a read-only cluster blocks it: patching the pod is a
    /// mutation).
    fn debug_state(&self) -> Option<Option<String>> {
        if !self.deps.exec.as_ref().is_some_and(ExecFlow::can_debug) {
            return None;
        }
        self.command_state(CommandId::POD_DEBUG)
    }

    /// Opens the debug dialog for this pod: reads the pod for its containers, then asks for the
    /// image, target and command. Does nothing on a read-only cluster (the button says why).
    pub fn open_debug(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.debug_state() != Some(None) {
            return;
        }
        let Some(flow) = self.deps.exec.clone() else {
            return;
        };
        self.exec_task = Some(flow.begin_debug(self.target.clone(), window, cx));
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
        let debug = self.debug_state().map(|blocked| {
            let tooltip = blocked
                .clone()
                .unwrap_or_else(|| "Add a debug container".to_owned());
            div().debug_selector(|| "detail-debug".to_owned()).child(
                Button::new("detail-debug")
                    .xsmall()
                    .ghost()
                    .icon(Icon::new(IconName::Bug).size(u(px(14.))))
                    .tooltip(tooltip)
                    .disabled(blocked.is_some())
                    .on_click(cx.listener(|this, _, window, cx| this.open_debug(window, cx))),
            )
        });
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
                .children(debug)
                .into_any_element(),
        )
    }
}
