//! [`ContainerPicker`]: the modal that asks which container of a pod to open a session in.

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _, Window, px,
};
use oxikube_app::ContainerChoices;
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::layout::{Selectable as _, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use oxikube_workspace::modal::{DIALOG_KEY_CONTEXT, ModalPlacement, ModalView};

use super::flow::{ExecFlow, ExecKind};

/// Asks which container to open a shell (or an attach) in. Up and Down move, Enter or a click
/// opens, Escape cancels (nothing is sent). The default container (or the one opened last in
/// this pod) starts selected, so Enter alone does what the command would have done.
pub struct ContainerPicker {
    kind: ExecKind,
    target: ResourceRef,
    choices: ContainerChoices,
    selected: usize,
    flow: ExecFlow,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for ContainerPicker {}

impl Focusable for ContainerPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for ContainerPicker {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }
}

impl ContainerPicker {
    /// A picker for `choices` of `target`, with `choices.preselected` selected.
    pub fn new(
        kind: ExecKind,
        target: ResourceRef,
        choices: ContainerChoices,
        flow: ExecFlow,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            kind,
            target,
            selected: choices.preselected,
            choices,
            flow,
            focus: cx.focus_handle(),
        }
    }

    /// Gives the picker the keyboard focus.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    /// The containers offered, in order.
    pub fn choices(&self) -> &ContainerChoices {
        &self.choices
    }

    /// The index of the selected container.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects the container `index` (out of range is ignored).
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.choices.containers.len() && index != self.selected {
            self.selected = index;
            cx.notify();
        }
    }

    /// Moves the selection by `delta`, stopping at the ends.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let last = self.choices.containers.len().saturating_sub(1);
        let next = self.selected.saturating_add_signed(delta).min(last);
        self.select(next, cx);
    }

    /// Opens a session in the container `index` and closes the picker.
    pub fn open(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(container) = self.choices.containers.get(index) else {
            return;
        };
        let name = container.name.to_string();
        self.flow
            .dispatch(self.kind, self.target.clone(), Some(name), cx);
        cx.emit(DismissEvent);
    }

    /// Opens a session in the selected container.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        self.open(self.selected, cx);
    }

    /// Closes the picker; nothing is sent.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "down" => self.move_selection(1, cx),
            "up" => self.move_selection(-1, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn title(&self) -> SharedString {
        format!("{} {}", self.kind.verb(), self.target.name).into()
    }
}

impl Render for ContainerPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let rows = self
            .choices
            .containers
            .iter()
            .enumerate()
            .map(|(index, container)| {
                let selected = index == self.selected;
                gpui::div()
                    .debug_selector(move || format!("container-row-{index}"))
                    .child(
                        Button::new(("container", index))
                            .label(container.label())
                            .selected(selected)
                            .w_full()
                            .on_click(cx.listener(move |this, _, _, cx| this.open(index, cx))),
                    )
            });
        v_flex()
            .id("container-picker")
            .debug_selector(|| "container-picker".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .w(u(px(420.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_key_down(cx.listener(|this, event, _, cx| this.on_key(event, cx)))
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(self.title())))
            .child(
                gpui::div()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child("The pod has several containers. Which one?"),
            )
            .child(v_flex().gap(u(px(4.))).children(rows))
            .child(
                DialogFooter::new()
                    .child(
                        Button::new("container-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        gpui::div()
                            .debug_selector(|| "container-open".into())
                            .child(
                                Button::new("container-open")
                                    .label("Open")
                                    .small()
                                    .primary()
                                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                            ),
                    ),
            )
    }
}
