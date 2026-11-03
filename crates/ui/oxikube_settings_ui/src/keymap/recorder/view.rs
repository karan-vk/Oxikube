//! [`KeybindRecorder`]: the modal that records a key binding, shows its conflicts and the
//! validator's verdict, and saves it as `keymap::SetBinding`.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    Keystroke, Subscription, Task, Window,
};
use oxikube_domain::command::Command;
use oxikube_keymap::edit::{
    ContextModel, is_modifier_only, keystroke_text, preview_edit, set_binding_operations,
};
use oxikube_keymap::{ActionRegistry, KeymapPlatform, KeymapRow, KnownNames};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::ModalPlacement;
use oxikube_workspace::{CommandDispatcher, ModalView};

use super::model::{ConflictChoice, RecorderModel};

/// How long the recorder waits after a keystroke before it takes the sequence as complete
/// (GPUI's own wait for the rest of a multi-key binding).
pub const IDLE_TIMEOUT: Duration = Duration::from_millis(1000);

/// How long edits settle before the validator runs on them.
pub const CHECK_DEBOUNCE: Duration = Duration::from_millis(150);

/// The modal. See the [module docs](super).
pub struct KeybindRecorder {
    pub(super) model: RecorderModel,
    platform: KeymapPlatform,
    pub(super) focus: FocusHandle,
    pub(super) context_input: Entity<InputState>,
    dispatcher: Rc<dyn CommandDispatcher>,
    rows: Vec<KeymapRow>,
    contexts: ContextModel,
    actions: ActionRegistry,
    user_text: String,
    /// Ends the sequence after a pause; replaced (so cancelled) by every keystroke.
    idle: Task<()>,
    /// The debounced validation; replaced by every change.
    check: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for KeybindRecorder {}

impl Focusable for KeybindRecorder {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for KeybindRecorder {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }

    fn dismiss_on_outside_click(&self, _: &App) -> bool {
        // A stray click must not throw away a recorded binding.
        false
    }
}

impl KeybindRecorder {
    /// A recorder for the action of `row`, recording at once, with keys written in `platform`'s
    /// modifier names. Every keystroke in this window goes to the recorder while it records, so
    /// no binding fires; Save sends `keymap::SetBinding` to `dispatcher`.
    pub fn new(
        row: &KeymapRow,
        platform: KeymapPlatform,
        dispatcher: Rc<dyn CommandDispatcher>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = RecorderModel::for_row(row);
        let context_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Everywhere");
            input.set_value(model.context().to_owned(), window, cx);
            input
        });
        let focus = cx.focus_handle();
        let weak = cx.entity().downgrade();
        let this_window = window.window_handle();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != this_window {
                return;
            }
            let keystroke = event.keystroke.clone();
            let captured = weak
                .update(cx, |this, cx| this.capture(&keystroke, cx))
                .unwrap_or(false);
            if captured {
                cx.stop_propagation();
            }
        });
        let input_events = cx.subscribe_in(&context_input, window, Self::on_context_input);
        Self {
            model,
            platform,
            focus,
            context_input,
            dispatcher,
            rows: oxikube_keymap::keymap_rows(cx),
            contexts: ContextModel::current(),
            actions: ActionRegistry::from_app(cx),
            user_text: oxikube_keymap::edit::user_text(cx),
            idle: Task::ready(()),
            check: Task::ready(()),
            _subscriptions: vec![intercept, input_events],
        }
    }

    /// The state, for tests and the render.
    pub fn model(&self) -> &RecorderModel {
        &self.model
    }

    /// Take `keystroke` when recording: returns whether it was consumed (no binding sees it).
    /// A modifier pressed alone is consumed but not recorded.
    pub fn capture(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        if !self.model.is_recording() {
            return false;
        }
        if is_modifier_only(keystroke) {
            return true;
        }
        let still_recording = self.model.push(keystroke_text(keystroke, self.platform));
        if still_recording {
            self.idle = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(IDLE_TIMEOUT).await;
                this.update(cx, |this, cx| {
                    this.model.stop();
                    cx.notify();
                })
                .ok();
            });
        }
        self.changed(cx);
        true
    }

    /// Record from nothing again (the Record button), with the focus on the recorder.
    pub fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model.restart();
        window.focus(&self.focus, cx);
        self.changed(cx);
    }

    /// Stop recording, keeping the keys.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.model.stop();
        cx.notify();
    }

    /// Forget the keys recorded (the Clear button).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.model.clear();
        self.changed(cx);
    }

    /// Answer the conflicts.
    pub fn choose(&mut self, choice: ConflictChoice, cx: &mut Context<Self>) {
        self.model.set_choice(choice);
        self.changed(cx);
    }

    /// Set the context expression (what typing in the field does).
    pub fn set_context(&mut self, context: &str, cx: &mut Context<Self>) {
        self.model.set_context(context);
        self.changed(cx);
    }

    /// Put `context` in the context field and use it, as if the user typed it.
    pub fn type_context(&mut self, context: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.context_input.update(cx, |input, cx| {
            input.set_value(context.to_owned(), window, cx)
        });
        self.set_context(context, cx);
    }

    /// Send the binding as `keymap::SetBinding` and close. Does nothing until
    /// [`RecorderModel::can_save`].
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.model.can_save() {
            return;
        }
        let Some(edit) = self.model.edit() else {
            return;
        };
        self.dispatcher
            .dispatch(Command::KeymapSetBinding { edit }, cx);
        cx.emit(DismissEvent);
    }

    /// Close without saving.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    /// The keys, the context or the answer changed: find the conflicts again now, and validate
    /// the edit once the changes settle, on the background executor.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.model.refresh_conflicts(&self.contexts, &self.rows);
        cx.notify();
        let Some(edit) = self.model.edit() else {
            self.check = Task::ready(());
            return;
        };
        let operations = match set_binding_operations(&edit) {
            Ok(operations) => operations,
            Err(err) => {
                self.model.set_preview(Err(err.to_string()));
                self.check = Task::ready(());
                return;
            }
        };
        let text = self.user_text.clone();
        let actions = self.actions.clone();
        let known = KnownNames::current();
        self.check = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECK_DEBOUNCE).await;
            let preview = cx
                .background_spawn(async move {
                    preview_edit(&text, &operations, &actions, &known).map_err(|e| e.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.model.set_preview(preview);
                cx.notify();
            })
            .ok();
        });
    }

    fn on_context_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                self.set_context(&text, cx);
            }
            // Typing a context is not a keystroke to record.
            InputEvent::Focus => self.stop(cx),
            InputEvent::Blur | InputEvent::PressEnter { .. } => {}
        }
    }
}
