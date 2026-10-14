//! [`NotificationService`]: the centre's public face, and its shared state ([`Inner`]).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::task::Waker;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::notification::NotificationKind;
use oxikube_ports::{ClockPort, StatePort};
use parking_lot::Mutex;

use super::centre::Centre;
use super::driver::{self, Watch};
use super::limiter::Limiter;
use super::model::{Notification, NotificationId, NotificationInput};
use super::mute;
use super::rules::NotificationRules;
use super::subscription::{Mailbox, NotificationsSubscription};
use crate::events::EventService;
use crate::session::ClusterSessionManager;
use crate::store::{Spawner, TaskGuard, spawn_guarded};

/// The in-app notification centre: Warning events of the watched kinds (and, through
/// [`submit`](Self::submit), any other source) through the rules, the per-cluster mute, the
/// rate limit and grouping, and the dedupe, into a bounded read/unread list. See the
/// [module docs](super).
///
/// Cheap to clone: a handle on shared state. Every method but the mute pair returns at once.
#[derive(Clone)]
pub struct NotificationService {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for NotificationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.inner.state.lock();
        f.debug_struct("NotificationService")
            .field("rows", &state.centre.len())
            .field("unread", &state.centre.unread())
            .field("muted", &state.muted.len())
            .finish_non_exhaustive()
    }
}

pub(crate) struct Inner {
    pub(crate) events: EventService,
    pub(crate) sessions: ClusterSessionManager,
    pub(crate) store: Arc<dyn StatePort>,
    pub(crate) clock: Arc<dyn ClockPort>,
    pub(crate) state: Mutex<State>,
    /// Held while the muted clusters are written, so two quick toggles land in order.
    pub(crate) persist: futures::lock::Mutex<()>,
}

pub(crate) struct State {
    pub(crate) rules: NotificationRules,
    pub(crate) centre: Centre,
    pub(crate) limiter: Limiter,
    pub(crate) muted: BTreeSet<ClusterId>,
    /// The kinds switched off: nothing of them is added.
    pub(crate) disabled_kinds: BTreeSet<NotificationKind>,
    /// The versions an update notification was added for (one per version).
    pub(crate) announced_versions: BTreeSet<String>,
    /// The muted clusters were read: until then no cluster is watched (a muted one must never
    /// toast at launch).
    pub(crate) loaded: bool,
    pub(crate) spawner: Option<Arc<dyn Spawner>>,
    follower: Option<TaskGuard>,
    pub(crate) watches: HashMap<ClusterId, Watch>,
    pub(crate) subscribers: HashMap<u64, Arc<Mailbox>>,
    next_subscriber: u64,
}

impl State {
    /// Whether an input of `kind` about `cluster` may be added: the kind is on and the cluster,
    /// if any, is not muted.
    pub(crate) fn accepts(&self, kind: NotificationKind, cluster: Option<&ClusterId>) -> bool {
        !self.disabled_kinds.contains(&kind) && !cluster.is_some_and(|c| self.muted.contains(c))
    }

    /// Tells every subscriber the centre changed (and that `raised` should toast); returns the
    /// wakers to wake once the lock is released.
    pub(crate) fn publish(&self, raised: Option<&Arc<Notification>>) -> Vec<Waker> {
        self.subscribers
            .values()
            .filter_map(|mailbox| mailbox.push(raised))
            .collect()
    }
}

pub(crate) fn wake(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}

impl NotificationService {
    /// A centre over `events` (the Warning events of the open `sessions`), remembering the muted
    /// clusters in `store` and timing its windows on `clock`. Nothing runs, and no cluster is
    /// watched, until [`load`](Self::load) read the muted clusters and
    /// [`attach`](Self::attach) (or [`start`](Self::start)) gave it a spawner.
    pub fn new(
        events: EventService,
        sessions: ClusterSessionManager,
        store: Arc<dyn StatePort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                events,
                sessions,
                store,
                clock,
                state: Mutex::new(State {
                    rules: NotificationRules::default(),
                    centre: Centre::default(),
                    limiter: Limiter::default(),
                    muted: BTreeSet::new(),
                    disabled_kinds: BTreeSet::new(),
                    announced_versions: BTreeSet::new(),
                    loaded: false,
                    spawner: None,
                    follower: None,
                    watches: HashMap::new(),
                    subscribers: HashMap::new(),
                    next_subscriber: 0,
                }),
                persist: futures::lock::Mutex::new(()),
            }),
        }
    }

    /// Reads the muted clusters, then watches the open, unmuted clusters. A store that cannot be
    /// read leaves nothing muted (and says so in the log). Runs once; later calls return at once.
    pub fn load(&self) -> BoxFuture<'static, ()> {
        let inner = self.inner.clone();
        async move {
            if inner.state.lock().loaded {
                return;
            }
            let stored = mute::load(&*inner.store).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "the muted clusters could not be read: none muted");
                BTreeSet::new()
            });
            {
                let mut state = inner.state.lock();
                state.muted.extend(stored);
                state.loaded = true;
            }
            driver::reconcile_all(&inner);
        }
        .boxed()
    }

    /// Lets the clusters' event pumps run on `spawner`. The first spawner stays.
    pub fn attach(&self, spawner: Arc<dyn Spawner>) {
        {
            let mut state = self.inner.state.lock();
            if state.spawner.is_some() {
                return;
            }
            state.spawner = Some(spawner);
        }
        driver::reconcile_all(&self.inner);
    }

    /// The loop that follows the sessions: a cluster opened is watched (unless muted), one
    /// closed is forgotten. It only waits, so a host runs it on its foreground executor.
    pub fn follow(&self) -> BoxFuture<'static, ()> {
        driver::follow(&self.inner).boxed()
    }

    /// [`load`](Self::load), [`follow`](Self::follow) and [`attach`](Self::attach) on
    /// `spawner`, for a host without an executor of its own (tests, headless runs). The
    /// follower is aborted with the service.
    pub fn start(&self, spawner: Arc<dyn Spawner>) {
        if self.inner.state.lock().follower.is_some() {
            return;
        }
        let load = self.load();
        let follow = self.follow();
        self.attach(spawner.clone());
        let follower = spawn_guarded(&spawner, async move {
            load.await;
            follow.await;
        });
        self.inner.state.lock().follower = Some(follower);
    }

    /// Hands the centre one occurrence from any source. Returns the row it landed in, or `None`
    /// when its kind is switched off or its cluster is muted. The typed hooks of the other
    /// sources ([`NotificationSourcePort`](oxikube_ports::NotificationSourcePort)) build their
    /// inputs and come here.
    pub fn submit(&self, input: NotificationInput) -> Option<Arc<Notification>> {
        self.submit_if(input, |_| true)
    }

    /// [`submit`](Self::submit) when `gate` agrees; `gate` runs under the state lock, after the
    /// kind and the cluster were checked, so a source can make its own check-and-remember atomic.
    pub(crate) fn submit_if(
        &self,
        input: NotificationInput,
        gate: impl FnOnce(&mut State) -> bool,
    ) -> Option<Arc<Notification>> {
        let now = self.inner.clock.now();
        let (row, wakers) = {
            let mut state = self.inner.state.lock();
            if !state.accepts(input.kind, input.cluster.as_ref()) || !gate(&mut state) {
                return None;
            }
            let State {
                centre,
                limiter,
                rules,
                ..
            } = &mut *state;
            let applied = centre.apply(input, limiter, rules, now);
            let wakers = state.publish(applied.raised.then_some(&applied.row));
            (applied.row, wakers)
        };
        wake(wakers);
        Some(row)
    }

    /// The rows, newest first.
    pub fn list(&self) -> Vec<Arc<Notification>> {
        self.inner.state.lock().centre.list()
    }

    /// The row `id`, while it is listed.
    pub fn get(&self, id: NotificationId) -> Option<Arc<Notification>> {
        self.inner.state.lock().centre.get(id)
    }

    /// How many rows are unread.
    pub fn unread(&self) -> usize {
        self.inner.state.lock().centre.unread()
    }

    /// Marks `id` read. Returns whether it was unread.
    pub fn mark_read(&self, id: NotificationId) -> bool {
        self.change(|state| state.centre.mark_read(id))
    }

    /// Marks every row read. Returns how many were unread.
    pub fn mark_all_read(&self) -> usize {
        let mut marked = 0;
        self.change(|state| {
            marked = state.centre.mark_all_read();
            marked > 0
        });
        marked
    }

    /// The rules in effect.
    pub fn rules(&self) -> NotificationRules {
        self.inner.state.lock().rules.clone()
    }

    /// Whether `cluster`'s raised notifications may reach the desktop (its `os_notifications`
    /// override; the user's opt-in is the OS service's).
    pub fn os_allowed(&self, cluster: &ClusterId) -> bool {
        self.inner.state.lock().rules.os_allowed(cluster)
    }

    /// Replaces the rules (the settings changed), atomically: a notification is judged by the old
    /// rules or the new, never a mix. A cluster whose watched kinds changed is subscribed again
    /// with the new kinds (an edit of a window or an ignored reason touches no subscription);
    /// nothing already listed changes.
    pub fn set_rules(&self, rules: NotificationRules) {
        {
            let mut state = self.inner.state.lock();
            if state.rules == rules {
                return;
            }
            let state = &mut *state;
            for (cluster, watch) in &mut state.watches {
                if state.rules.event_filter(cluster) != rules.event_filter(cluster) {
                    watch.pump = None;
                }
            }
            state.rules = rules;
        }
        driver::reconcile_all(&self.inner);
    }

    /// Subscribes to the centre's changes.
    pub fn subscribe(&self) -> NotificationsSubscription {
        let mailbox = Arc::new(Mailbox::new());
        let mut state = self.inner.state.lock();
        state.next_subscriber += 1;
        let id = state.next_subscriber;
        state.subscribers.insert(id, mailbox.clone());
        NotificationsSubscription {
            inner: Arc::downgrade(&self.inner),
            id,
            mailbox,
        }
    }

    /// Whether `cluster`'s events are watched for notifications now (tests and diagnostics).
    pub fn is_watching(&self, cluster: &ClusterId) -> bool {
        self.inner
            .state
            .lock()
            .watches
            .get(cluster)
            .is_some_and(|watch| watch.pump.is_some())
    }

    /// Runs `change` on the state and tells the subscribers when it returns true.
    pub(crate) fn change(&self, change: impl FnOnce(&mut State) -> bool) -> bool {
        let (changed, wakers) = {
            let mut state = self.inner.state.lock();
            let changed = change(&mut state);
            let wakers = if changed {
                state.publish(None)
            } else {
                Vec::new()
            };
            (changed, wakers)
        };
        wake(wakers);
        changed
    }
}
