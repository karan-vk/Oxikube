//! [`SourcesModel`]: what the sources screen shows. Plain Rust, no GPUI.

use gpui::SharedString;
use oxikube_app::sources::SourceRow;
use oxikube_ports::SourceState;

/// How the first read of the rows went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    /// Nothing has arrived yet.
    Loading,
    /// The rows are in.
    Ready,
    /// They could not be read (the message is shown).
    Failed(String),
}

/// A one-line message under the header: what the last action did, or why it failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// The text.
    pub text: SharedString,
    /// Shown as an error.
    pub error: bool,
}

/// The rows, the load state and the last notice.
#[derive(Debug)]
pub struct SourcesModel {
    rows: Vec<SourceRow>,
    load: LoadState,
    notice: Option<Notice>,
}

impl Default for SourcesModel {
    fn default() -> Self {
        Self::new()
    }
}

impl SourcesModel {
    /// A model that is still loading.
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            load: LoadState::Loading,
            notice: None,
        }
    }

    /// Replaces the rows.
    pub fn set_rows(&mut self, rows: Vec<SourceRow>) {
        self.rows = rows;
        self.load = LoadState::Ready;
    }

    /// The rows could not be read.
    pub fn set_failed(&mut self, message: String) {
        self.load = LoadState::Failed(message);
    }

    /// Sets (or clears) the notice.
    pub fn set_notice(&mut self, notice: Option<Notice>) {
        self.notice = notice;
    }

    /// The rows, in list order.
    pub fn rows(&self) -> &[SourceRow] {
        &self.rows
    }

    /// The row at `ix`.
    pub fn row(&self, ix: usize) -> Option<&SourceRow> {
        self.rows.get(ix)
    }

    /// The load state.
    pub fn load_state(&self) -> &LoadState {
        &self.load
    }

    /// The notice, if any.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// How many sources have a problem (missing, unreadable, invalid).
    pub fn error_count(&self) -> usize {
        self.rows.iter().filter(|row| row.is_error()).count()
    }

    /// "3 sources, 1 with a problem": the header's summary.
    pub fn summary(&self) -> String {
        let count = self.rows.len();
        let errors = self.error_count();
        let sources = format!("{count} source{}", if count == 1 { "" } else { "s" });
        match errors {
            0 => sources,
            n => format!("{sources}, {n} with a problem"),
        }
    }
}

/// How a row's status reads: "3 contexts", "File not found", "Not read yet".
pub fn status_text(row: &SourceRow) -> String {
    let contexts = |n: usize| format!("{n} context{}", if n == 1 { "" } else { "s" });
    match (row.state, &row.message) {
        (None, _) => "Not read yet".to_owned(),
        (Some(SourceState::Found), None) => contexts(row.contexts),
        (Some(SourceState::Found), Some(note)) => format!("{}, {note}", contexts(row.contexts)),
        (Some(SourceState::Blank), message) => message.clone().unwrap_or_else(|| "Empty".into()),
        (Some(SourceState::Missing), message) => {
            message.clone().unwrap_or_else(|| "Not found".into())
        }
        (Some(SourceState::Unreadable), message) => message
            .clone()
            .unwrap_or_else(|| "Could not be read".into()),
        (Some(SourceState::Invalid), message) => message
            .clone()
            .unwrap_or_else(|| "Not a valid kubeconfig".into()),
    }
}
