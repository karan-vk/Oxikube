//! The two small pieces every fake is built from: [`Script`] (queued responses) and
//! [`CallLog`] (recorded calls), plus [`Timeline`] for streams replayed against a clock.
//!
//! A fake keeps one [`Script`] per port method and one [`CallLog`] for the whole port.
//! A test queues responses with [`Script::push_ok`] / [`Script::push_err`]; each call
//! records itself and pops the next queued response. When the queue is empty the fake
//! falls back to its configured state or, where it has none, to the error built by
//! [`unscripted`], so a test never passes by accident on a response it did not script.
//!
//! Everything here is guarded by `parking_lot::Mutex` and is runtime-agnostic: no tokio,
//! no OS threads, no real sleeps.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::{self, BoxStream};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::ClockPort;
use parking_lot::Mutex;

/// The error a fake returns when a method is called with nothing scripted and no
/// fallback state: `Internal`, naming the port and the method.
pub fn unscripted(port: &str, method: &str) -> OxiError {
    OxiError::internal(format!("{port}::{method}: no scripted response"))
}

/// A FIFO queue of scripted responses for one port method.
///
/// Responses are consumed once, in the order they were pushed (`OxiError` is not
/// `Clone`, so a response cannot be replayed).
pub struct Script<T> {
    queue: Mutex<VecDeque<OxiResult<T>>>,
}

impl<T> Default for Script<T> {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
        }
    }
}

impl<T> fmt::Debug for Script<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Script")
            .field("queued", &self.len())
            .finish()
    }
}

impl<T> Script<T> {
    /// Queues one response.
    pub fn push(&self, response: OxiResult<T>) -> &Self {
        self.queue.lock().push_back(response);
        self
    }

    /// Queues one successful response.
    pub fn push_ok(&self, value: T) -> &Self {
        self.push(Ok(value))
    }

    /// Queues one failed response.
    pub fn push_err(&self, error: OxiError) -> &Self {
        self.push(Err(error))
    }

    /// Takes the next queued response, if any.
    pub fn pop(&self) -> Option<OxiResult<T>> {
        self.queue.lock().pop_front()
    }

    /// Takes the next queued response, or computes the fallback when the queue is empty.
    pub fn next_or_else(&self, fallback: impl FnOnce() -> OxiResult<T>) -> OxiResult<T> {
        self.pop().unwrap_or_else(fallback)
    }

    /// Takes the next queued response, or [`unscripted`]`(port, method)` when the queue
    /// is empty.
    pub fn next_or_unscripted(&self, port: &str, method: &str) -> OxiResult<T> {
        self.next_or_else(|| Err(unscripted(port, method)))
    }

    /// Number of responses still queued.
    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }

    /// `true` when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.queue.lock().is_empty()
    }

    /// Drops every queued response.
    pub fn clear(&self) {
        self.queue.lock().clear();
    }
}

/// An append-only record of the calls made on a fake, in call order.
pub struct CallLog<C> {
    calls: Mutex<Vec<C>>,
}

impl<C> Default for CallLog<C> {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl<C: fmt::Debug> fmt::Debug for CallLog<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.calls.lock().iter()).finish()
    }
}

impl<C> CallLog<C> {
    /// Appends one call.
    pub fn record(&self, call: C) {
        self.calls.lock().push(call);
    }

    /// Number of recorded calls.
    pub fn len(&self) -> usize {
        self.calls.lock().len()
    }

    /// `true` when no call was recorded.
    pub fn is_empty(&self) -> bool {
        self.calls.lock().is_empty()
    }

    /// Removes and returns every recorded call.
    pub fn take(&self) -> Vec<C> {
        std::mem::take(&mut *self.calls.lock())
    }

    /// Forgets every recorded call.
    pub fn clear(&self) {
        self.calls.lock().clear();
    }
}

impl<C: Clone> CallLog<C> {
    /// A copy of every recorded call, in call order.
    pub fn calls(&self) -> Vec<C> {
        self.calls.lock().clone()
    }
}

/// A scripted stream: items delivered at offsets from the moment the stream is created,
/// measured on a [`ClockPort`] (normally [`FakeClockPort`](crate::FakeClockPort)).
///
/// Replay never sleeps for real: each item waits on `ClockPort::sleep_until`, so with a
/// fake clock the test decides when time passes (`FakeClockPort::advance`). Items with
/// equal offsets are delivered in the order they were added; items are sorted by offset
/// (stably) before replay. After the last item the stream ends, unless
/// [`keep_open`](Self::keep_open) was set, in which case it stays pending like a live
/// watch.
pub struct Timeline<T> {
    events: Vec<(Duration, OxiResult<T>)>,
    keep_open: bool,
}

impl<T> Default for Timeline<T> {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            keep_open: false,
        }
    }
}

impl<T> fmt::Debug for Timeline<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Timeline")
            .field(
                "offsets",
                &self.events.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
            )
            .field("keep_open", &self.keep_open)
            .finish()
    }
}

impl<T: Send + 'static> Timeline<T> {
    /// An empty timeline (ends immediately unless kept open).
    pub fn new() -> Self {
        Self::default()
    }

    /// A timeline that delivers `items` at offset zero, in order.
    pub fn immediate(items: impl IntoIterator<Item = T>) -> Self {
        items
            .into_iter()
            .fold(Self::new(), |t, item| t.ok_at(Duration::ZERO, item))
    }

    /// Adds a response at `offset` from stream creation.
    #[must_use]
    pub fn at(mut self, offset: Duration, item: OxiResult<T>) -> Self {
        self.events.push((offset, item));
        self
    }

    /// Adds a successful item at `offset`.
    #[must_use]
    pub fn ok_at(self, offset: Duration, item: T) -> Self {
        self.at(offset, Ok(item))
    }

    /// Adds an error item at `offset`.
    #[must_use]
    pub fn err_at(self, offset: Duration, error: OxiError) -> Self {
        self.at(offset, Err(error))
    }

    /// Keeps the stream pending after the last item instead of ending it.
    #[must_use]
    pub fn keep_open(mut self) -> Self {
        self.keep_open = true;
        self
    }

    /// Number of scripted items.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// `true` when no item is scripted.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Turns the timeline into a stream measured on `clock`. Offsets count from the
    /// moment this is called (`clock.now()`), not from the first poll.
    pub fn replay(self, clock: Arc<dyn ClockPort>) -> BoxStream<'static, OxiResult<T>> {
        let start = clock.now();
        let mut events = self.events;
        events.sort_by_key(|(offset, _)| *offset);
        let items = stream::unfold(
            (clock, events.into_iter()),
            move |(clock, mut events)| async move {
                let (offset, item) = events.next()?;
                let deadline = start.checked_add(offset).unwrap_or(jiff::Timestamp::MAX);
                clock.sleep_until(deadline).await;
                Some((item, (clock, events)))
            },
        );
        // Fused, so polling after the end keeps returning `None` instead of panicking.
        if self.keep_open {
            items.chain(stream::pending()).boxed()
        } else {
            items.fuse().boxed()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FakeClockPort;
    use futures::FutureExt;
    use futures::executor::block_on;

    #[test]
    fn script_pops_in_order_then_falls_back() {
        let script: Script<u32> = Script::default();
        script.push_ok(1).push_err(OxiError::not_found("gone"));
        assert_eq!(script.len(), 2);
        assert_eq!(script.pop().map(Result::ok), Some(Some(1)));
        assert!(script.pop().is_some_and(|r| r.is_err()));
        assert!(script.is_empty());
        assert_eq!(script.next_or_else(|| Ok(7)).ok(), Some(7));
        let err = script.next_or_unscripted("FakeX", "get").unwrap_err();
        assert!(err.message().contains("FakeX::get"));
        script.push_ok(3);
        script.clear();
        assert!(script.is_empty());
    }

    #[test]
    fn call_log_records_in_order() {
        let log: CallLog<&str> = CallLog::default();
        assert!(log.is_empty());
        log.record("a");
        log.record("b");
        assert_eq!(log.calls(), vec!["a", "b"]);
        assert_eq!(log.len(), 2);
        assert_eq!(log.take(), vec!["a", "b"]);
        assert!(log.is_empty());
        log.record("c");
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn timeline_sorts_by_offset_and_waits_on_the_clock() {
        let clock = Arc::new(FakeClockPort::default());
        let mut s = Timeline::new()
            .ok_at(Duration::from_secs(2), "late")
            .ok_at(Duration::ZERO, "first")
            .ok_at(Duration::from_secs(2), "late-2")
            .replay(clock.clone());
        assert_eq!(
            s.next().now_or_never().flatten().map(Result::ok),
            Some(Some("first"))
        );
        assert!(s.next().now_or_never().is_none());
        clock.advance(Duration::from_secs(2));
        assert_eq!(block_on(s.next()).map(Result::ok), Some(Some("late")));
        assert_eq!(block_on(s.next()).map(Result::ok), Some(Some("late-2")));
        assert!(block_on(s.next()).is_none());
    }

    #[test]
    fn kept_open_timeline_stays_pending() {
        let clock = Arc::new(FakeClockPort::default());
        let mut s = Timeline::immediate([1, 2])
            .keep_open()
            .replay(clock.clone());
        assert_eq!(block_on(s.next()).map(Result::ok), Some(Some(1)));
        assert_eq!(block_on(s.next()).map(Result::ok), Some(Some(2)));
        assert!(s.next().now_or_never().is_none());
        clock.advance(Duration::from_secs(3600));
        assert!(s.next().now_or_never().is_none());
    }
}
