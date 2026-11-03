//! [`Controls`]: what a settings page keeps for its controls.

use std::collections::{HashMap, HashSet};

use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, SharedString, Subscription, Task, Window,
};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::SettingsStore;
use oxikube_ui::input::{InputEvent, InputState};
use serde_json::Value;

use super::list::ListRows;
use super::spec::ControlSpec;
use super::validate::FieldError;
use crate::page::{PageField, SettingsPage, SettingsPageModel};

/// What a write targets: a field of the user file (`None`) or of one cluster's block. Debounce
/// and in-flight state are keyed by it so the same field of two clusters never share a slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct WriteKey {
    pub(super) cluster: Option<ClusterId>,
    pub(super) path: SharedString,
}

impl WriteKey {
    pub(super) fn new(cluster: Option<&ClusterId>, path: &SharedString) -> Self {
        Self {
            cluster: cluster.cloned(),
            path: path.clone(),
        }
    }
}

/// The editing state of one field.
pub(super) enum Editor {
    /// A text, number, colour or font input.
    Text {
        input: Entity<InputState>,
        _subscription: Subscription,
    },
    /// A list of strings.
    List(ListRows),
}

/// The controls of one settings page: the input entities (made when a field is first shown),
/// the inline errors, and the debounce and write bookkeeping. Owned by the page entity, so
/// dropping the page cancels every pending write timer; the tasks held here are replaced or
/// dropped only from outside.
#[derive(Default)]
pub struct Controls {
    pub(super) editors: HashMap<SharedString, Editor>,
    pub(super) errors: HashMap<SharedString, FieldError>,
    /// The debounce timer of each field with a typed value waiting to be written.
    pub(super) pending: HashMap<WriteKey, Task<()>>,
    /// Fields whose typed value has not reached the file yet.
    pub(super) dirty: HashSet<WriteKey>,
    /// Fields with a write in flight, and the newest value waiting behind it.
    pub(super) in_flight: HashSet<WriteKey>,
    pub(super) queued: HashMap<WriteKey, Value>,
    /// Fields whose input must show the store's value on the next change even if focused (after
    /// a reset).
    pub(super) resync: HashSet<SharedString>,
    pub(super) writes: Vec<Task<()>>,
    pub(super) fonts: Option<Vec<SharedString>>,
}

impl Controls {
    /// The inline error under the field at `path`, if its input is not acceptable.
    pub fn error(&self, path: &str) -> Option<&FieldError> {
        self.errors.get(path)
    }

    /// Whether a typed value for `path` is still waiting for its debounce or its write.
    pub fn has_unsaved(&self, path: &str) -> bool {
        self.dirty.iter().any(|key| key.path == path)
            || self.in_flight.iter().any(|key| key.path == path)
            || self.errors.contains_key(path)
    }

    /// The input entity of the field at `path`, once it has been shown.
    pub fn input(&self, path: &str) -> Option<&Entity<InputState>> {
        match self.editors.get(path)? {
            Editor::Text { input, .. } => Some(input),
            Editor::List(_) => None,
        }
    }

    /// The installed font families, asked of the text system once.
    pub(super) fn fonts(&mut self, window: &Window) -> &[SharedString] {
        self.fonts.get_or_insert_with(|| {
            let mut names: Vec<SharedString> = window
                .text_system()
                .all_font_names()
                .into_iter()
                .filter(|name| !name.starts_with('.'))
                .map(SharedString::from)
                .collect();
            names.sort();
            names.dedup();
            names
        })
    }

    /// Makes the input entities of `fields` that have none yet, showing the value in effect.
    pub(crate) fn ensure(
        &mut self,
        fields: &[PageField],
        cluster: Option<&ClusterId>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) {
        for field in fields {
            if self.editors.contains_key(&field.path) {
                continue;
            }
            let editor = match &field.control {
                ControlSpec::Font => {
                    self.fonts(window);
                    Some(text_editor(field, cluster, window, cx))
                }
                ControlSpec::Number(_) | ControlSpec::Text | ControlSpec::Color => {
                    Some(text_editor(field, cluster, window, cx))
                }
                ControlSpec::List => Some(Editor::List(ListRows::new(field, cluster, window, cx))),
                ControlSpec::Switch | ControlSpec::Choice(_) | ControlSpec::Json => None,
            };
            if let Some(editor) = editor {
                self.editors.insert(field.path.clone(), editor);
            }
        }
    }

    /// Follows the store: every editor that is not being edited shows the value in effect.
    pub(crate) fn sync_from_store(
        &mut self,
        model: &SettingsPageModel,
        cluster: Option<&ClusterId>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) {
        for field in model.sections().iter().flat_map(|section| &section.fields) {
            let path = &field.path;
            if !self.editors.contains_key(path) {
                continue;
            }
            // A value being typed (waiting for its pause, or rejected) is the person's, not the
            // file's; a write in flight is not: its echo shows the same value.
            if self.dirty.contains(&WriteKey::new(cluster, path)) || self.errors.contains_key(path)
            {
                continue;
            }
            let forced = self.resync.remove(path);
            let effective = effective_value(path, cluster, cx);
            match self.editors.get_mut(path) {
                Some(Editor::Text { input, .. }) => {
                    let shown = display_text(effective.as_ref());
                    let input = input.clone();
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    if input.read(cx).value().as_ref() != shown && (forced || !focused) {
                        input.update(cx, |input, cx| input.set_value(shown, window, cx));
                    }
                }
                Some(Editor::List(rows)) => rows.show(effective.as_ref(), field, window, cx),
                None => {}
            }
        }
    }

    /// Forgets every input and error, so the fields shown next are built from the values of the
    /// cluster (or the file) the page now points at. Writes in flight and typed values waiting
    /// for their pause are kept: each is keyed by the cluster it was typed for.
    pub(crate) fn forget_editors(&mut self) {
        self.editors.clear();
        self.errors.clear();
        self.resync.clear();
    }

    /// Drops the error of `path` (a valid value was typed, or the field was reset).
    pub(super) fn clear_error(&mut self, path: &str) {
        self.errors.remove(path);
    }
}

fn text_editor(
    field: &PageField,
    cluster: Option<&ClusterId>,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) -> Editor {
    let initial = display_text(effective_value(&field.path, cluster, cx).as_ref());
    let input = cx.new(|cx| {
        let mut state = InputState::new(window, cx);
        state.set_value(initial, window, cx);
        state
    });
    let path = field.path.clone();
    let spec = field.control.clone();
    let subscription = cx.subscribe_in(
        &input,
        window,
        move |page, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                let cluster = page.cluster.clone();
                page.controls.typed(&path, &spec, &text, cluster, cx);
            }
        },
    );
    Editor::Text {
        input,
        _subscription: subscription,
    }
}

/// The value in effect for `path` at `cluster`, from the store global.
pub(super) fn effective_value(path: &str, cluster: Option<&ClusterId>, cx: &App) -> Option<Value> {
    cx.try_global::<SettingsStore>()?
        .effective_value(path, cluster)
        .cloned()
}

/// A value as the text of its input: strings bare, everything else as JSON, `null` blank.
pub(super) fn display_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}
