//! The tab as plain data: [`Stage`], the preview it showed and the output log.

use std::collections::VecDeque;

use oxikube_app::InstallPreview;
use oxikube_app::guard::ConfirmationRequest;
use oxikube_domain::helm::{OutputKind, OutputLine};

use super::form::InstallForm;

/// How many output lines the log keeps (the oldest go first).
pub const MAX_LOG_LINES: usize = 10_000;

/// Where the install is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Looking for `helm` and reading the chart's defaults.
    Loading,
    /// No usable `helm` binary: the tab explains what to do and offers nothing else.
    NoHelm,
    /// The form and the values are being edited.
    Editing,
    /// The dry run is running.
    Previewing,
    /// Waiting for the guard's confirmation request.
    Asking,
    /// The guard asks: the tab shows its text with Install and Cancel.
    Confirming(ConfirmationRequest),
    /// `helm install` is running; its output streams into the log.
    Installing,
    /// The release was installed; the handler's message.
    Installed(String),
    /// The install failed or was refused; why.
    Failed(String),
}

impl Stage {
    /// Whether the form and the editor take input (nothing is running or asking).
    pub fn is_editable(&self) -> bool {
        matches!(self, Self::Editing | Self::Failed(_))
    }
}

/// A preview and what it was made from, so an edit since makes it stale.
#[derive(Debug, Clone)]
pub struct Shown {
    /// The preview.
    pub preview: InstallPreview,
    /// The form it was made from.
    pub form: InstallForm,
    /// The editor version of the values it was made from.
    pub values_version: u64,
}

impl Shown {
    /// Whether the form and values are still those previewed.
    pub fn is_fresh(&self, form: &InstallForm, values_version: u64) -> bool {
        &self.form == form && self.values_version == values_version
    }
}

/// The output of the running (or last) install, bounded.
#[derive(Debug, Clone, Default)]
pub struct OutputLog {
    lines: VecDeque<OutputLine>,
    dropped: usize,
}

impl OutputLog {
    /// Appends `line`, dropping the oldest past [`MAX_LOG_LINES`].
    pub fn push(&mut self, line: OutputLine) {
        if self.lines.len() == MAX_LOG_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    /// The `index`th line kept.
    pub fn get(&self, index: usize) -> Option<&OutputLine> {
        self.lines.get(index)
    }

    /// How many lines are kept.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// `true` before the first line.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// How many old lines were dropped.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// The kept lines as text, one per line (for the tests and the copy action).
    pub fn text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            if line.kind == OutputKind::Stderr {
                out.push_str("! ");
            }
            out.push_str(&line.text);
            out.push('\n');
        }
        out
    }

    /// Forgets every line.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.dropped = 0;
    }
}
