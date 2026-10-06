//! Helpers for tests of the sources screen and of the crates built on it (feature
//! `test-support`): a backend that serves fixed rows and records the commands it was sent, and
//! sample rows.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use futures::stream::{self, BoxStream};
use gpui::{App, Task};
use oxikube_app::command_bus::CommandOutput;
use oxikube_app::sources::SourceRow;
use oxikube_domain::OxiResult;
use oxikube_domain::command::Command;
use oxikube_ports::{SourceState, SourcesChanged, UserSource};

use super::backend::SourcesBackend;

/// Where [`ScriptedBackend`] says pasted kubeconfigs are stored.
pub const STORED_DIR: &str = "/config/kubeconfigs";

/// A [`SourcesBackend`] with fixed rows that records every command: the "fake bus" of the
/// screen's tests. Cheap to clone; clones share the rows and the record.
#[derive(Clone, Default)]
pub struct ScriptedBackend {
    rows: Rc<RefCell<Vec<SourceRow>>>,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl ScriptedBackend {
    /// A backend serving `rows`.
    pub fn new(rows: Vec<SourceRow>) -> Self {
        Self {
            rows: Rc::new(RefCell::new(rows)),
            sent: Rc::default(),
        }
    }

    /// The commands sent so far, oldest first.
    pub fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    /// Replaces the rows the next read returns.
    pub fn set_rows(&self, rows: Vec<SourceRow>) {
        *self.rows.borrow_mut() = rows;
    }
}

impl SourcesBackend for ScriptedBackend {
    fn rows(&self, _: &mut App) -> Task<OxiResult<Vec<SourceRow>>> {
        Task::ready(Ok(self.rows.borrow().clone()))
    }

    fn run(&self, command: Command, _: &mut App) -> Task<OxiResult<CommandOutput>> {
        self.sent.borrow_mut().push(command);
        Task::ready(Ok(CommandOutput::message("done")))
    }

    fn deletes_file(&self, source: &UserSource) -> bool {
        source
            .path
            .as_deref()
            .is_some_and(|path| oxikube_app::sources::is_stored_in(STORED_DIR.as_ref(), path))
    }

    fn stored_dir(&self) -> PathBuf {
        PathBuf::from(STORED_DIR)
    }

    fn changes(&self) -> BoxStream<'static, SourcesChanged> {
        Box::pin(stream::pending())
    }
}

/// A row that was read: `contexts` contexts, no problem.
pub fn found(source: UserSource, contexts: usize) -> SourceRow {
    row(source, Some(SourceState::Found), contexts, None)
}

/// A row with a problem: `state` and the `message` shown next to it.
pub fn broken(source: UserSource, state: SourceState, message: &str) -> SourceRow {
    row(source, Some(state), 0, Some(message))
}

/// A row with every field given.
pub fn row(
    source: UserSource,
    state: Option<SourceState>,
    contexts: usize,
    message: Option<&str>,
) -> SourceRow {
    let stored = oxikube_app::sources::is_stored_in(
        STORED_DIR.as_ref(),
        source.path.as_deref().unwrap_or_else(|| "".as_ref()),
    );
    let label = match &source.path {
        Some(path) => path.display().to_string(),
        None => "KUBECONFIG or ~/.kube/config".to_owned(),
    };
    SourceRow {
        source,
        label,
        stored,
        state,
        contexts,
        message: message.map(Into::into),
    }
}
