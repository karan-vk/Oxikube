//! [`Progress`]: how a long-running handler reports each item as it finishes, and learns that its
//! caller wants it to stop.
//!
//! A command is plain data and its handler answers once, with one [`CommandOutput`]. A handler
//! that works through many objects (a multi-object apply, E20-S03) can also report every object
//! as it is done, so a view fills its table while the work goes on. The caller puts a
//! [`Progress`] in the [`DispatchContext`](super::DispatchContext); the guard hands it to the
//! handler ([`HandlerContext::report`](super::HandlerContext::report)). A dispatch without one
//! (an agent's tool call) gets the same final output and loses nothing but the early rows.
//!
//! Reports are JSON, like a [`CommandOutput`]'s data, so the bus stays untyped; they carry names
//! and statuses, never an object's body. The [`CancellationToken`] is how the caller asks the
//! handler to stop between items: the handler reports what it did not do and returns normally,
//! so everything it did is still audited. Dropping the dispatch future instead stops it at once,
//! mid-request.
//!
//! [`CommandOutput`]: super::CommandOutput

use std::fmt;
use std::sync::Arc;

use oxikube_ports::CancellationToken;
use serde_json::Value;

/// A sink for a handler's per-item reports and the token that asks it to stop. Cheap to clone;
/// two are equal when they are the same sink.
#[derive(Clone)]
pub struct Progress {
    inner: Arc<Inner>,
}

struct Inner {
    sink: Box<dyn Fn(Value) + Send + Sync>,
    cancel: CancellationToken,
}

impl Progress {
    /// Reports go to `sink` (called on the handler's task, so it must not block); `cancel` asks
    /// the handler to stop.
    pub fn new(sink: impl Fn(Value) + Send + Sync + 'static, cancel: CancellationToken) -> Self {
        Self {
            inner: Arc::new(Inner {
                sink: Box::new(sink),
                cancel,
            }),
        }
    }

    /// Hands `report` to the sink.
    pub fn report(&self, report: Value) {
        (self.inner.sink)(report);
    }

    /// Whether the caller asked the handler to stop.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancel.is_cancelled()
    }
}

impl fmt::Debug for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Progress")
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl PartialEq for Progress {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for Progress {}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    #[test]
    fn reports_reach_the_sink_and_cancel_is_seen() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let cancel = CancellationToken::new();
        let progress = Progress::new(
            move |report| sink.lock().unwrap().push(report),
            cancel.clone(),
        );
        progress.report(json!({"n": 1}));
        assert!(!progress.is_cancelled());
        cancel.cancel();
        let clone = progress.clone();
        assert!(clone.is_cancelled(), "a clone shares the token");
        assert_eq!(*seen.lock().unwrap(), [json!({"n": 1})]);
        assert_eq!(progress, clone);
        let other = Progress::new(|_| {}, CancellationToken::new());
        assert_ne!(progress, other);
    }
}
