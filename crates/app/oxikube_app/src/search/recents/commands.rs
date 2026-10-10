//! [`StateRecents`]: the commands run lately, kept through the state store.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oxikube_domain::command::CommandId;
use oxikube_ports::{StateKey, StatePort};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::list::RecentList;
use super::writeback::Writeback;
use crate::command_bus::{RECENTS_CAPACITY, RecentsStore};

/// The state key of the command recents.
pub const RECENTS_KEY: &str = "recents.commands";

/// The version written with the list, so a later shape can be told apart.
const VERSION: u64 = 1;

/// A [`RecentsStore`] over [`StatePort`]: in memory for every call the palette makes, written
/// behind it. See the [module docs](super) for what is kept and when it is written.
///
/// Build one with [`StateRecents::new`], run [`load`](Self::load) once to bring in the previous
/// run's list and keep [`run_writer`](Self::run_writer) running on the runtime.
pub struct StateRecents {
    state: Arc<dyn StatePort>,
    key: StateKey,
    list: Mutex<RecentList<CommandId>>,
    /// The stored list was read (or there was none) and merged. Until then nothing is written, so
    /// a failed read cannot lead to the stored list being replaced by this run's alone.
    loaded: AtomicBool,
    writeback: Writeback,
    /// One write at a time, so the latest list is the last one written.
    writing: tokio::sync::Mutex<()>,
}

impl StateRecents {
    /// Recents over `state`, empty until [`load`](Self::load) runs.
    pub fn new(state: Arc<dyn StatePort>) -> Self {
        Self {
            state,
            key: StateKey::new(RECENTS_KEY).expect("a valid state key"),
            list: Mutex::new(RecentList::new(RECENTS_CAPACITY)),
            loaded: AtomicBool::new(false),
            writeback: Writeback::default(),
            writing: tokio::sync::Mutex::new(()),
        }
    }

    /// Reads the stored list and puts it behind whatever was run since the app started. A missing
    /// value is an empty list; an unreadable one (the wrong shape) too, and ids that no longer
    /// name a registered command are dropped. A store that cannot be read leaves the recents in
    /// memory, logs once and holds back writes: [`flush`](Self::flush) tries the read again first,
    /// so the stored list is never replaced by one that was not merged with it.
    pub async fn load(&self) {
        if self.loaded.load(Ordering::Acquire) {
            return;
        }
        let stored = match self.state.kv_get(&self.key).await {
            Ok(Some(value)) => match decode(&value) {
                Some(ids) => ids,
                None => {
                    // Nothing usable is stored: what is written next replaces it.
                    self.writeback
                        .failed("the stored recents are unreadable", "bad shape");
                    self.loaded.store(true, Ordering::Release);
                    return;
                }
            },
            Ok(None) => {
                self.loaded.store(true, Ordering::Release);
                return;
            }
            Err(error) => {
                self.writeback
                    .failed("the recents could not be read", error.kind());
                return;
            }
        };
        self.list.lock().append_older(stored);
        self.loaded.store(true, Ordering::Release);
    }

    /// Writes the recents now if they changed since the last write. The app calls it as it quits.
    /// A failure keeps the change for the next attempt and is logged once. When the stored list
    /// has not been read yet it is read first; if that fails again nothing is written.
    pub async fn flush(&self) {
        let _writing = self.writing.lock().await;
        if !self.writeback.take() {
            return;
        }
        self.load().await;
        if !self.loaded.load(Ordering::Acquire) {
            self.writeback.retry_later();
            return;
        }
        let value = encode(&self.list.lock());
        match self.state.kv_set(&self.key, value).await {
            Ok(()) => self.writeback.healthy(),
            Err(error) => {
                self.writeback.retry_later();
                self.writeback
                    .failed("the recents could not be saved", error.kind());
            }
        }
    }

    /// How many failure lines were logged (the tests assert "once").
    #[cfg(test)]
    pub(super) fn logged(&self) -> usize {
        self.writeback.logged()
    }

    /// Whether a change is waiting to be written.
    pub fn is_dirty(&self) -> bool {
        self.writeback.is_dirty()
    }

    /// Writes the recents after each change, for as long as the future is polled: waits for a
    /// change, calls `pause` with [`DEBOUNCE`](super::DEBOUNCE) so a burst of commands is one write, then
    /// [`flush`](Self::flush)es. `pause` is the runtime's timer (the caller owns the runtime).
    /// Hold the task that runs it; dropping the task stops it.
    pub async fn run_writer<P, F>(&self, pause: P)
    where
        P: Fn(Duration) -> F,
        F: Future<Output = ()>,
    {
        loop {
            self.writeback.settled(&pause).await;
            self.flush().await;
        }
    }
}

impl RecentsStore for StateRecents {
    fn recent(&self) -> Vec<CommandId> {
        self.list.lock().iter().copied().collect()
    }

    fn record(&self, id: CommandId) {
        if self.list.lock().touch(id) {
            self.writeback.mark();
        }
    }

    fn clear(&self) {
        if self.list.lock().clear() {
            self.writeback.mark();
        }
    }
}

impl std::fmt::Debug for StateRecents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateRecents")
            .field("recent", &self.list.lock().len())
            .field("dirty", &self.writeback.is_dirty())
            .finish()
    }
}

fn encode(list: &RecentList<CommandId>) -> Value {
    let ids: Vec<&str> = list.iter().map(|id| id.as_str()).collect();
    json!({ "v": VERSION, "ids": ids })
}

/// The registered commands in a stored value, latest first; `None` when it is not the shape
/// [`encode`] writes. Ids that are no longer registered are skipped.
fn decode(value: &Value) -> Option<Vec<CommandId>> {
    let ids = value.get("ids")?.as_array()?;
    Some(
        ids.iter()
            .filter_map(Value::as_str)
            .filter_map(|id| id.parse::<CommandId>().ok())
            .collect(),
    )
}
