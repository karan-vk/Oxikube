//! [`LogSession`]: the owner's handle on one stream, and [`LogReader`], a shared read-only view
//! of it.

use std::sync::Arc;

use oxikube_ports::LogOptions;

use super::delta::LogDeltas;
use super::ring::LogBuffer;
use super::shared::Shared;
use super::state::LogState;
use super::target::LogTarget;
use crate::store::TaskGuard;

/// A read-only view of a session: its buffer, state and deltas. Cheap to clone; it does not keep
/// the stream open (only the [`LogSession`] does), so the service's listing, an agent tool or a
/// second view can hold one freely.
#[derive(Clone)]
pub struct LogReader {
    shared: Arc<Shared>,
}

impl LogReader {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    /// The service-unique id of the session.
    pub fn id(&self) -> u64 {
        self.shared.id
    }

    /// What the session reads.
    pub fn target(&self) -> &LogTarget {
        &self.shared.target
    }

    /// The options the stream was opened with (`container` filled in from the target).
    pub fn options(&self) -> &LogOptions {
        &self.shared.options
    }

    /// Where the session is in its life.
    pub fn state(&self) -> LogState {
        self.shared.state()
    }

    /// Runs `f` over the retained lines and the state, as one consistent snapshot.
    ///
    /// The session's lock is held while `f` runs: copy out what a frame shows (a range of
    /// [`LogBuffer::range`], cloned) and return; do not render or wait inside it.
    pub fn read<R>(&self, f: impl FnOnce(&LogBuffer, &LogState) -> R) -> R {
        self.shared.read(f)
    }

    /// Lines retained now.
    pub fn len(&self) -> usize {
        self.read(|buffer, _| buffer.len())
    }

    /// Whether no line is retained.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Batches the session has committed so far: the unit its readers are woken in (one per
    /// flush tick or full batch, not one per line).
    pub fn batches(&self) -> u64 {
        self.shared.batches()
    }

    /// A new stream of the session's batched deltas, starting from nothing seen: its first delta
    /// carries every line retained at that point. Each call has its own cursor.
    pub fn deltas(&self) -> LogDeltas {
        LogDeltas::new(self.shared.clone())
    }
}

impl std::fmt::Debug for LogReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogReader")
            .field("id", &self.shared.id)
            .field("target", &self.shared.target)
            .finish_non_exhaustive()
    }
}

/// One open log stream and its bounded buffer. Dropping it cancels the stream: the task that reads
/// it is aborted (abort-on-drop), which drops the port's stream and closes the connection, and
/// the session's [`LogDeltas`] end. It reads through [`LogReader`]; see [`LogService::open`].
///
/// [`LogService::open`]: super::LogService::open
pub struct LogSession {
    reader: LogReader,
    _task: TaskGuard,
}

impl LogSession {
    pub(super) fn new(shared: Arc<Shared>, task: TaskGuard) -> Self {
        Self {
            reader: LogReader::new(shared),
            _task: task,
        }
    }

    /// A shared read-only view that does not keep the stream open.
    pub fn reader(&self) -> LogReader {
        self.reader.clone()
    }
}

impl std::ops::Deref for LogSession {
    type Target = LogReader;

    fn deref(&self) -> &LogReader {
        &self.reader
    }
}

impl Drop for LogSession {
    fn drop(&mut self) {
        // The guard aborts the task after this; readers kept elsewhere see a cancelled session.
        self.reader.shared.close();
    }
}

impl std::fmt::Debug for LogSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.reader.fmt(f)
    }
}
