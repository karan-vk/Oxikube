//! [`JumpHistory`]: what was typed into the `:` jump bar, per cluster.

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ids::ClusterId;
use oxikube_domain::redact::redact;
use oxikube_ports::{StateKey, StatePort};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::list::RecentList;
use super::writeback::Writeback;

/// The state key of a cluster's history is this, then the cluster id (`history.jump/<id>`).
pub const JUMP_KEY_PREFIX: &str = "history.jump/";

/// How many jumps are kept per cluster.
pub const JUMP_CAPACITY: usize = 100;

/// The longest text that is remembered, in characters. A longer line is not a jump.
pub const JUMP_TEXT_MAX_CHARS: usize = 200;

/// The version written with a list.
const VERSION: u64 = 1;

/// One cluster's history in memory.
struct History {
    list: RecentList<String>,
    /// The stored list has been read and merged.
    loaded: bool,
    /// Changed since it was written.
    dirty: bool,
}

/// The jump bar's history: the lines run lately in each cluster, latest first, a line once.
///
/// In memory for every call the bar makes, written behind it like
/// [`StateRecents`](super::StateRecents) (same writer, same failure handling). The bar calls
/// [`load`](Self::load) when a cluster's tab opens, [`record`](Self::record) after a jump ran and
/// [`recent`](Self::recent) to complete from.
pub struct JumpHistory {
    state: Arc<dyn StatePort>,
    clusters: Mutex<HashMap<ClusterId, History>>,
    writeback: Writeback,
    writing: tokio::sync::Mutex<()>,
}

impl JumpHistory {
    /// An empty history over `state`.
    pub fn new(state: Arc<dyn StatePort>) -> Self {
        Self {
            state,
            clusters: Mutex::new(HashMap::new()),
            writeback: Writeback::default(),
            writing: tokio::sync::Mutex::new(()),
        }
    }

    /// Remembers that `text` (what followed the `:`) was run in `cluster`. Surrounding space is
    /// dropped and runs of space become one, so `pod  app=x` and `pod app=x` are one entry; it
    /// moves to the front when it is there already.
    ///
    /// Returns whether it was remembered: blank text, text longer than [`JUMP_TEXT_MAX_CHARS`]
    /// and text that looks like it holds a secret are not.
    pub fn record(&self, cluster: &ClusterId, text: &str) -> bool {
        let Some(text) = normalise(text) else {
            return false;
        };
        let mut clusters = self.clusters.lock();
        let history = clusters.entry(cluster.clone()).or_insert_with(History::new);
        if history.list.touch(text) {
            history.dirty = true;
            self.writeback.mark();
        }
        true
    }

    /// What was run in `cluster`, latest first.
    pub fn recent(&self, cluster: &ClusterId) -> Vec<String> {
        self.clusters
            .lock()
            .get(cluster)
            .map(|history| history.list.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Forgets the history of `cluster`.
    pub fn clear(&self, cluster: &ClusterId) {
        let mut clusters = self.clusters.lock();
        let history = clusters.entry(cluster.clone()).or_insert_with(History::new);
        if history.list.clear() {
            history.dirty = true;
            self.writeback.mark();
        }
    }

    /// Reads the stored history of `cluster` and puts it behind what was run since. Once per
    /// cluster: later calls do nothing. A store that cannot be read leaves the history in memory,
    /// logs once, and the next call tries again; a value of the wrong shape counts as empty.
    pub async fn load(&self, cluster: &ClusterId) {
        if self.clusters.lock().get(cluster).is_some_and(|h| h.loaded) {
            return;
        }
        let Some(key) = key_of(cluster) else {
            return;
        };
        let stored = match self.state.kv_get(&key).await {
            Ok(Some(value)) => decode(&value).unwrap_or_else(|| {
                self.writeback
                    .failed("the stored jump history is unreadable", "bad shape");
                Vec::new()
            }),
            Ok(None) => Vec::new(),
            Err(error) => {
                self.writeback
                    .failed("the jump history could not be read", error.kind());
                return;
            }
        };
        let mut clusters = self.clusters.lock();
        let history = clusters.entry(cluster.clone()).or_insert_with(History::new);
        history.list.append_older(stored);
        history.loaded = true;
    }

    /// Writes the histories that changed. The app calls it as it quits. A failed one is kept for
    /// the next attempt and logged once.
    pub async fn flush(&self) {
        let _writing = self.writing.lock().await;
        if !self.writeback.take() {
            return;
        }
        let pending: Vec<(ClusterId, Value)> = self
            .clusters
            .lock()
            .iter_mut()
            .filter(|(_, history)| history.dirty)
            .map(|(cluster, history)| {
                history.dirty = false;
                (cluster.clone(), encode(&history.list))
            })
            .collect();
        for (cluster, value) in pending {
            let Some(key) = key_of(&cluster) else {
                continue;
            };
            match self.state.kv_set(&key, value).await {
                Ok(()) => self.writeback.healthy(),
                Err(error) => {
                    if let Some(history) = self.clusters.lock().get_mut(&cluster) {
                        history.dirty = true;
                    }
                    self.writeback.retry_later();
                    self.writeback
                        .failed("the jump history could not be saved", error.kind());
                }
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

    /// Writes after each change, like [`StateRecents::run_writer`](super::StateRecents::run_writer).
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

impl History {
    fn new() -> Self {
        Self {
            list: RecentList::new(JUMP_CAPACITY),
            loaded: false,
            dirty: false,
        }
    }
}

impl std::fmt::Debug for JumpHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JumpHistory")
            .field("clusters", &self.clusters.lock().len())
            .field("dirty", &self.writeback.is_dirty())
            .finish()
    }
}

fn key_of(cluster: &ClusterId) -> Option<StateKey> {
    StateKey::new(format!("{JUMP_KEY_PREFIX}{cluster}")).ok()
}

/// The text to remember, or `None` when it is not worth (or not safe) to keep.
fn normalise(text: &str) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || text.chars().count() > JUMP_TEXT_MAX_CHARS {
        return None;
    }
    // Anything the redaction patterns would change looks like a credential: do not keep it.
    matches!(redact(&text), Cow::Borrowed(_)).then_some(text)
}

fn encode(list: &RecentList<String>) -> Value {
    let jumps: Vec<&str> = list.iter().map(String::as_str).collect();
    json!({ "v": VERSION, "jumps": jumps })
}

/// The lines in a stored value, latest first, re-checked as if they were typed now; `None` when
/// the value is not the shape [`encode`] writes.
fn decode(value: &Value) -> Option<Vec<String>> {
    let jumps = value.get("jumps")?.as_array()?;
    Some(
        jumps
            .iter()
            .filter_map(Value::as_str)
            .filter_map(normalise)
            .collect(),
    )
}
