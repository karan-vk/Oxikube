//! [`LabelEditor`]: the rows of the label editor, the labels / annotations switch, and what they
//! make: the validated edits and the preview of one sample object.

use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use oxikube_domain::selection::{
    LabelEdit, LabelField, LabelPreview, MAX_EDITS, preview_edits, validate_edits,
};
use oxikube_ui::input::InputState;

use super::dialog::BulkDialog;

/// One row of the editor: a key, a value, and whether the key is removed instead of set.
pub struct EditorRow {
    pub(super) key: Entity<InputState>,
    pub(super) value: Entity<InputState>,
    /// Remove the key from every object instead of setting it.
    pub remove: bool,
}

/// What the label editor holds. The dialog owns one for `selection::Label`.
pub struct LabelEditor {
    /// Whether the edits are to labels or annotations.
    pub field: LabelField,
    pub(super) rows: Vec<EditorRow>,
    /// The sample object's name, for the preview's heading.
    pub sample_name: String,
    sample_labels: Vec<(String, String)>,
    sample_annotations: Vec<(String, String)>,
    _subscriptions: Vec<Subscription>,
}

impl LabelEditor {
    /// An editor with one empty row, previewing against the sample's current labels and
    /// annotations.
    pub(super) fn new(
        sample_name: String,
        sample_labels: Vec<(String, String)>,
        sample_annotations: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<BulkDialog>,
    ) -> Self {
        let mut editor = Self {
            field: LabelField::Labels,
            rows: Vec::new(),
            sample_name,
            sample_labels,
            sample_annotations,
            _subscriptions: Vec::new(),
        };
        editor.add_row(window, cx);
        editor
    }

    /// Adds an empty row, as the "Add" button does. Does nothing past [`MAX_EDITS`] rows.
    pub fn add_row(&mut self, window: &mut Window, cx: &mut Context<BulkDialog>) {
        if self.rows.len() >= MAX_EDITS {
            return;
        }
        let key =
            cx.new(|cx| InputState::new(window, cx).placeholder("key (app.kubernetes.io/name)"));
        let value = cx.new(|cx| InputState::new(window, cx).placeholder("value"));
        for input in [&key, &value] {
            self._subscriptions
                .push(cx.subscribe_in(input, window, BulkDialog::on_typed));
        }
        self.rows.push(EditorRow {
            key,
            value,
            remove: false,
        });
    }

    /// Drops row `ix`; the editor always keeps one row.
    pub(super) fn drop_row(&mut self, ix: usize) {
        if self.rows.len() > 1 && ix < self.rows.len() {
            self.rows.remove(ix);
        }
    }

    /// Flips row `ix` between setting its key and removing it, as its "Remove" button does.
    pub fn toggle_remove(&mut self, ix: usize) {
        if let Some(row) = self.rows.get_mut(ix) {
            row.remove = !row.remove;
        }
    }

    /// How many rows the editor has.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Whether row `ix` removes its key.
    pub fn removes(&self, ix: usize) -> bool {
        self.rows.get(ix).is_some_and(|row| row.remove)
    }

    /// Fills row `ix` as the user typing would.
    pub fn fill(
        &mut self,
        ix: usize,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<BulkDialog>,
    ) {
        if let Some(row) = self.rows.get(ix) {
            row.key
                .update(cx, |input, cx| input.set_value(key.to_owned(), window, cx));
            row.value.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
        }
    }

    /// The edits the rows make now: a row with a key sets it (to the value, which may be empty) or
    /// removes it; a row with no key is ignored. Keys and label values are trimmed.
    pub fn edits(&self, cx: &App) -> Vec<LabelEdit> {
        self.rows
            .iter()
            .filter_map(|row| {
                let key = row.key.read(cx).value().trim().to_owned();
                if key.is_empty() {
                    return None;
                }
                if row.remove {
                    return Some(LabelEdit::remove(key));
                }
                let value = row.value.read(cx).value();
                let value = match self.field {
                    LabelField::Labels => value.trim().to_owned(),
                    LabelField::Annotations => value.to_string(),
                };
                Some(LabelEdit::set(key, value))
            })
            .collect()
    }

    /// What is wrong with the edits, in the words of the apiserver's rules; `None` for good ones or
    /// none yet.
    pub fn problem(&self, cx: &App) -> Option<String> {
        let edits = self.edits(cx);
        if edits.is_empty() {
            return None;
        }
        validate_edits(self.field, &edits)
            .err()
            .map(|error| error.message().to_owned())
    }

    /// Whether the edits can be sent: at least one, and valid.
    pub fn is_ready(&self, cx: &App) -> bool {
        let edits = self.edits(cx);
        !edits.is_empty() && validate_edits(self.field, &edits).is_ok()
    }

    /// The sample object's labels or annotations after the edits, every key marked.
    pub fn preview(&self, cx: &App) -> Vec<LabelPreview> {
        let current = match self.field {
            LabelField::Labels => &self.sample_labels,
            LabelField::Annotations => &self.sample_annotations,
        };
        preview_edits(
            current.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            &self.edits(cx),
        )
    }
}
