//! [`AggregateView`]: the read-only side of an aggregate's bookkeeping (streams, pod events,
//! hidden sources), and [`AggregateChanges`], the stream that says when any of it changed.
//!
//! The lines are in the session's [`LogBuffer`](crate::logs::LogBuffer) like any session's; this
//! is everything else a multi-pod viewer shows: which pods and containers are being read, what
//! changed in the pod set, how many pods were left out by the stream cap, and which sources the
//! user switched off.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use futures::Stream;
use parking_lot::Mutex;

use super::sources::{HiddenSources, PodChange, PodEvent, SourceId, SourceInfo, SourceState};

/// Pod events kept for readers that were not looking (the oldest are forgotten).
const EVENT_LOG: usize = 64;

/// What the driver writes and the views read.
pub(in crate::logs) struct AggShared {
    label: String,
    inner: Mutex<Inner>,
}

struct Inner {
    selector: Option<String>,
    sources: Vec<SourceInfo>,
    hidden: HiddenSources,
    hidden_version: u64,
    events: VecDeque<PodEvent>,
    next_event: u64,
    skipped_pods: usize,
    /// Pods the selector matches now, whether or not a container of them can be read yet.
    matched_pods: usize,
    /// Bumped by every change: what [`AggregateChanges`] compares its cursor with.
    version: u64,
    wakers: Vec<Waker>,
}

impl AggShared {
    pub(in crate::logs) fn new(label: String) -> Self {
        Self {
            label,
            inner: Mutex::new(Inner {
                selector: None,
                sources: Vec::new(),
                hidden: HiddenSources::default(),
                hidden_version: 0,
                events: VecDeque::new(),
                next_event: 0,
                skipped_pods: 0,
                matched_pods: 0,
                version: 0,
                wakers: Vec::new(),
            }),
        }
    }

    /// Applies `change` under the lock, then wakes the readers once.
    fn change<R>(&self, change: impl FnOnce(&mut Inner) -> R) -> R {
        let (result, wakers) = {
            let mut inner = self.inner.lock();
            let result = change(&mut inner);
            inner.version += 1;
            (result, std::mem::take(&mut inner.wakers))
        };
        for waker in wakers {
            waker.wake();
        }
        result
    }

    pub(super) fn set_selector(&self, selector: String) {
        self.change(|inner| inner.selector = Some(selector));
    }

    pub(super) fn add_source(&self, info: SourceInfo) {
        self.change(|inner| inner.sources.push(info));
    }

    pub(super) fn set_source_state(&self, id: SourceId, state: SourceState) {
        self.change(|inner| {
            if let Some(source) = inner.sources.iter_mut().find(|s| s.id == id) {
                source.state = state;
            }
        });
    }

    pub(super) fn push_event(&self, pod: Arc<str>, change: PodChange) {
        self.change(|inner| {
            let seq = inner.next_event;
            inner.next_event += 1;
            inner.events.push_back(PodEvent { seq, pod, change });
            if inner.events.len() > EVENT_LOG {
                inner.events.pop_front();
            }
        });
    }

    pub(super) fn skipped_pods(&self) -> usize {
        self.inner.lock().skipped_pods
    }

    /// Publishes how many pods match and how many of them the cap left out; wakes the readers
    /// only when either changed.
    pub(super) fn set_pod_counts(&self, matched: usize, skipped: usize) {
        let unchanged = {
            let inner = self.inner.lock();
            inner.matched_pods == matched && inner.skipped_pods == skipped
        };
        if !unchanged {
            self.change(|inner| {
                inner.matched_pods = matched;
                inner.skipped_pods = skipped;
            });
        }
    }
}

/// The aggregate's streams, pod events and hidden sources, as one read-only handle. Cheap to
/// clone; it does not keep the streams open (the [`AggregateSession`](super::AggregateSession)
/// does).
#[derive(Clone)]
pub struct AggregateView {
    shared: Arc<AggShared>,
}

impl AggregateView {
    pub(in crate::logs) fn new(shared: Arc<AggShared>) -> Self {
        Self { shared }
    }

    /// What is read, as kubectl names it: `deployment/api`, `selector app=web`.
    pub fn label(&self) -> &str {
        &self.shared.label
    }

    /// The label selector the pods are matched by, once it is resolved (`None` while the
    /// object is being read, or when it could not be).
    pub fn selector(&self) -> Option<String> {
        self.shared.inner.lock().selector.clone()
    }

    /// Every stream opened so far, in the order they were opened.
    pub fn sources(&self) -> Vec<SourceInfo> {
        self.shared.inner.lock().sources.clone()
    }

    /// Pods matched by the selector but left out because `logs.max_streams` streams are
    /// running: the "N more pods not streamed" notice.
    pub fn skipped_pods(&self) -> usize {
        self.shared.skipped_pods()
    }

    /// Pods the selector matches now, including those with no container to read yet (still
    /// `ContainerCreating` or `Pending`).
    pub fn matched_pods(&self) -> usize {
        self.shared.inner.lock().matched_pods
    }

    /// The pod events with a seq above `after` (all of them for `None`), oldest first. Only the
    /// newest 64 are kept.
    pub fn events_after(&self, after: Option<u64>) -> Vec<PodEvent> {
        let inner = self.shared.inner.lock();
        inner
            .events
            .iter()
            .filter(|event| after.is_none_or(|seq| event.seq > seq))
            .cloned()
            .collect()
    }

    /// What the user switched off, with a counter that changes whenever it does.
    pub fn hidden(&self) -> (HiddenSources, u64) {
        let inner = self.shared.inner.lock();
        (inner.hidden.clone(), inner.hidden_version)
    }

    /// Replaces what is switched off (a viewer that reopened the session keeps the user's choice).
    pub fn set_hidden(&self, hidden: HiddenSources) {
        self.shared.change(|inner| {
            inner.hidden = hidden;
            inner.hidden_version += 1;
        });
    }

    /// Hides or shows `container` of `pod` (the whole pod when `None`) in the view. The stream
    /// keeps running either way.
    pub fn toggle_source(&self, pod: &str, container: Option<&str>) {
        self.shared.change(|inner| {
            inner.hidden.toggle(pod, container);
            inner.hidden_version += 1;
        });
    }

    /// A stream of change notifications, starting from the state at this call: it yields once for
    /// each change (coalesced: a reader that is slow sees one). It never ends by itself; the
    /// reader drops it (a view drops its pump task with the session). Each call has its own
    /// cursor.
    pub fn changes(&self) -> AggregateChanges {
        let version = self.shared.inner.lock().version;
        AggregateChanges {
            shared: self.shared.clone(),
            seen: version,
        }
    }
}

impl std::fmt::Debug for AggregateView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AggregateView")
            .field("label", &self.shared.label)
            .finish_non_exhaustive()
    }
}

/// Says when an aggregate's streams, pod events or hidden sources changed; see
/// [`AggregateView::changes`]. The item is the new version number.
pub struct AggregateChanges {
    shared: Arc<AggShared>,
    seen: u64,
}

impl Stream for AggregateChanges {
    type Item = u64;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<u64>> {
        let this = self.get_mut();
        let mut inner = this.shared.inner.lock();
        if inner.version != this.seen {
            this.seen = inner.version;
            return Poll::Ready(Some(inner.version));
        }
        if !inner.wakers.iter().any(|w| w.will_wake(cx.waker())) {
            inner.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}
