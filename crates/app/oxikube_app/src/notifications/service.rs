//! [`NotificationService`]: the centre's public face, and its shared state ([`Inner`]).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::task::Waker;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
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
    store: Arc<dyn StatePort>,
    pub(crate) clock: Arc<dyn ClockPort>,
    pub(crate) state: Mutex<State>,
    /// Held while the muted clusters are written, so two quick toggles land in order.
    persist: futures::lock::Mutex<()>,
}

pub(crate) struct State {
    pub(crate) rules: NotificationRules,
    pub(crate) centre: Centre,
    pub(crate) limiter: Limiter,
    pub(crate) muted: BTreeSet<ClusterId>,
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
    /// when its cluster is muted.
    pub fn submit(&self, input: NotificationInput) -> Option<Arc<Notification>> {
        let now = self.inner.clock.now();
        let (row, wakers) = {
            let mut state = self.inner.state.lock();
            if state.muted.contains(&input.cluster) {
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
        self.change(|state| state.centre.mark_all_read())
    }

    /// Whether `cluster` is muted.
    pub fn is_muted(&self, cluster: &ClusterId) -> bool {
        self.inner.state.lock().muted.contains(cluster)
    }

    /// The muted clusters.
    pub fn muted(&self) -> Vec<ClusterId> {
        self.inner.state.lock().muted.iter().cloned().collect()
    }

    /// Mutes `cluster`: its events stop being watched for notifications (its feed may still run
    /// for the views) and nothing about it is added; the rows already listed stay. Remembered
    /// across restarts. Returns whether it was unmuted.
    ///
    /// # Errors
    ///
    /// The state store's error when the change could not be saved (it still applies until the
    /// app quits).
    pub async fn mute(&self, cluster: &ClusterId) -> OxiResult<bool> {
        let changed = self.change(|state| {
            let changed = state.muted.insert(cluster.clone());
            if changed {
                state.watches.remove(cluster);
                state.limiter.forget(cluster);
            }
            changed
        });
        if changed {
            self.persist().await?;
        }
        Ok(changed)
    }

    /// Unmutes `cluster`: notifying resumes with what happens from now on. Returns whether it
    /// was muted.
    ///
    /// # Errors
    ///
    /// As for [`mute`](Self::mute).
    pub async fn unmute(&self, cluster: &ClusterId) -> OxiResult<bool> {
        let changed = self.change(|state| state.muted.remove(cluster));
        if changed {
            driver::reconcile(&self.inner, cluster);
            self.persist().await?;
        }
        Ok(changed)
    }

    /// The rules in effect.
    pub fn rules(&self) -> NotificationRules {
        self.inner.state.lock().rules.clone()
    }

    /// Replaces the rules (the settings changed). The clusters' subscriptions are opened again
    /// with the new kinds; nothing already listed changes.
    pub fn set_rules(&self, rules: NotificationRules) {
        {
            let mut state = self.inner.state.lock();
            if state.rules == rules {
                return;
            }
            state.rules = rules;
            for watch in state.watches.values_mut() {
                watch.pump = None;
            }
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

    /// Runs `change` on the state and tells the subscribers when it returns something truthy.
    fn change<R: Changed>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let (result, wakers) = {
            let mut state = self.inner.state.lock();
            let result = change(&mut state);
            let wakers = if result.changed() {
                state.publish(None)
            } else {
                Vec::new()
            };
            (result, wakers)
        };
        wake(wakers);
        result
    }

    async fn persist(&self) -> OxiResult<()> {
        let _in_order = self.inner.persist.lock().await;
        let muted = self.inner.state.lock().muted.clone();
        mute::save(&*self.inner.store, &muted).await
    }
}

/// Whether a state change is worth telling the subscribers about.
trait Changed {
    fn changed(&self) -> bool;
}

impl Changed for bool {
    fn changed(&self) -> bool {
        *self
    }
}

impl Changed for usize {
    fn changed(&self) -> bool {
        *self > 0
    }
}
