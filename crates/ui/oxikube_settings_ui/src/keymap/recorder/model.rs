//! [`RecorderModel`]: what the key recorder holds and decides, as plain data (tested without a
//! window): the keys recorded so far, the context, the conflicts and the user's answer to them,
//! the validator's verdict on the edit, and the `keymap::SetBinding` payload a save sends.

use oxikube_domain::command::KeymapBindingEdit;
use oxikube_keymap::edit::{ChainConflict, ContextModel, EditPreview, ref_of_row};
use oxikube_keymap::{KeymapRow, RowStatus};

/// The most keystrokes one binding records (`ctrl-k ctrl-s` is two); the recording stops there.
pub const MAX_KEYSTROKES: usize = 4;

/// What to do with the bindings the new one clashes with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConflictChoice {
    /// Leave them: where both apply the later layer wins, which is the user's binding. A conflict
    /// that is [`overwritten`](oxikube_keymap::edit::ChainConflict::overwritten) is not left: the
    /// new binding takes its place in `keymap.json` whatever the answer.
    #[default]
    KeepBoth,
    /// Take them out of force: a user binding is deleted, a default one unbound with `null`.
    Replace,
}

/// The validator's verdict on the edit as it stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Not checked yet (nothing recorded, or the check is running).
    Pending,
    /// The edit can be written; these are its warnings.
    Writable(Vec<String>),
    /// The edit must not be written; these are the errors it would add (or why it cannot be
    /// made at all).
    Blocked(Vec<String>),
}

/// The recorder's state: the keys, the context, the conflicts and the answer, the verdict.
#[derive(Clone, Debug)]
pub struct RecorderModel {
    action: String,
    title: String,
    data: Option<String>,
    editing: Option<KeymapRow>,
    keystrokes: Vec<String>,
    recording: bool,
    context: String,
    conflicts: Vec<ChainConflict>,
    choice: ConflictChoice,
    verdict: Verdict,
}

impl RecorderModel {
    /// A recorder for the action of `row`: a bound row is the binding being changed (its context
    /// is the starting one), a command bound nowhere gets a binding added. Recording starts at
    /// once.
    pub fn for_row(row: &KeymapRow) -> Self {
        let editing =
            (row.status != RowStatus::Unbound && !row.keystrokes.is_empty()).then(|| row.clone());
        Self {
            action: row.action.clone(),
            title: row.title.clone(),
            data: row.data.clone(),
            context: row.context.clone().unwrap_or_default(),
            editing,
            keystrokes: Vec::new(),
            recording: true,
            conflicts: Vec::new(),
            choice: ConflictChoice::default(),
            verdict: Verdict::Pending,
        }
    }

    /// The action's registered name.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// What a person reads for the action.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The binding being changed, when the recorder was opened on one.
    pub fn editing(&self) -> Option<&KeymapRow> {
        self.editing.as_ref()
    }

    /// The keystrokes recorded, in `keymap.json` spelling.
    pub fn keystrokes(&self) -> &[String] {
        &self.keystrokes
    }

    /// The keystrokes joined by spaces.
    pub fn keystrokes_text(&self) -> String {
        self.keystrokes.join(" ")
    }

    /// Whether keys pressed now are recorded.
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// The context expression typed (blank: everywhere).
    pub fn context(&self) -> &str {
        &self.context
    }

    /// The context as the edit writes it.
    pub fn context_expr(&self) -> Option<&str> {
        Some(self.context.trim()).filter(|context| !context.is_empty())
    }

    /// The bindings in force the new one clashes with.
    pub fn conflicts(&self) -> &[ChainConflict] {
        &self.conflicts
    }

    /// Whether the answer to the conflicts changes the edit: some conflict can be kept beside the
    /// new binding. When every one is overwritten (a user binding of the same keys in the same
    /// context), "Keep both" and "Replace them" would write the same file.
    pub fn has_choice(&self) -> bool {
        self.conflicts.iter().any(|conflict| !conflict.overwritten)
    }

    /// The conflicts the save overwrites whatever the answer.
    pub fn overwritten(&self) -> impl Iterator<Item = &ChainConflict> {
        self.conflicts
            .iter()
            .filter(|conflict| conflict.overwritten)
    }

    /// The answer to the conflicts.
    pub fn choice(&self) -> ConflictChoice {
        self.choice
    }

    /// The validator's verdict.
    pub fn verdict(&self) -> &Verdict {
        &self.verdict
    }

    /// Record one more keystroke. Returns whether the recording goes on (it stops at
    /// [`MAX_KEYSTROKES`]).
    pub fn push(&mut self, keystroke: String) -> bool {
        if !self.recording {
            return false;
        }
        self.keystrokes.push(keystroke);
        self.verdict = Verdict::Pending;
        if self.keystrokes.len() >= MAX_KEYSTROKES {
            self.recording = false;
        }
        self.recording
    }

    /// Start recording again from nothing.
    pub fn restart(&mut self) {
        self.keystrokes.clear();
        self.conflicts.clear();
        self.recording = true;
        self.verdict = Verdict::Pending;
    }

    /// Stop recording; the keys recorded stay.
    pub fn stop(&mut self) {
        self.recording = false;
    }

    /// Forget the keys recorded, without recording again.
    pub fn clear(&mut self) {
        self.keystrokes.clear();
        self.conflicts.clear();
        self.recording = false;
        self.verdict = Verdict::Pending;
    }

    /// Change the context expression.
    pub fn set_context(&mut self, context: &str) {
        if self.context != context {
            self.context = context.to_owned();
            self.verdict = Verdict::Pending;
        }
    }

    /// Answer the conflicts.
    pub fn set_choice(&mut self, choice: ConflictChoice) {
        if self.choice != choice {
            self.choice = choice;
            self.verdict = Verdict::Pending;
        }
    }

    /// Find the conflicts of the keys and context as they stand among `rows`.
    pub fn refresh_conflicts(&mut self, contexts: &ContextModel, rows: &[KeymapRow]) {
        self.conflicts = contexts.conflicts(
            &self.action,
            self.context_expr(),
            &self.keystrokes_text(),
            rows,
            self.editing.as_ref(),
        );
    }

    /// Record the validator's result for the edit as it stands.
    pub fn set_preview(&mut self, preview: Result<EditPreview, String>) {
        self.verdict = match preview {
            Err(message) => Verdict::Blocked(vec![message]),
            Ok(preview) if preview.is_writable() => {
                Verdict::Writable(preview.warnings().map(|d| d.message()).collect())
            }
            Ok(preview) => {
                Verdict::Blocked(preview.introduced.iter().map(|d| d.message()).collect())
            }
        };
    }

    /// Whether the recorded binding is the one already in force (nothing to save).
    pub fn is_unchanged(&self) -> bool {
        self.editing.as_ref().is_some_and(|row| {
            row.keystrokes == self.keystrokes && row.context.as_deref() == self.context_expr()
        })
    }

    /// Whether Save does something now: keys recorded, a change, and the validator agrees.
    pub fn can_save(&self) -> bool {
        !self.keystrokes.is_empty()
            && !self.is_unchanged()
            && matches!(self.verdict, Verdict::Writable(_))
    }

    /// The `keymap::SetBinding` payload of the binding as it stands (`None` with no keys).
    pub fn edit(&self) -> Option<KeymapBindingEdit> {
        if self.keystrokes.is_empty() {
            return None;
        }
        let unbind = match self.choice {
            ConflictChoice::KeepBoth => Vec::new(),
            ConflictChoice::Replace => self
                .conflicts
                .iter()
                .filter_map(|conflict| ref_of_row(&conflict.row))
                .collect(),
        };
        Some(KeymapBindingEdit {
            action: self.action.clone(),
            data: self.data.clone(),
            context: self.context_expr().map(str::to_owned),
            keystrokes: self.keystrokes_text(),
            replaces: self.editing.as_ref().and_then(ref_of_row),
            unbind,
        })
    }
}
