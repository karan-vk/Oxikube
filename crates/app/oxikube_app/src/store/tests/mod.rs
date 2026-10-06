//! Store tests over `FakeResourcePort` / `FakeTableFeedPort` scripts, on a deterministic
//! executor (no runtime, no threads) and the fake clock.

mod budget;
mod feeds;
mod filter;
mod order;
mod props;
mod refcount;
mod registry;
mod scope;

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt;
use futures::{FutureExt, StreamExt};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline, pod};
use parking_lot::Mutex;

use super::{
    ResourceStore, RowChange, StoreConfig, StoreDelta, StoreObject, StoreOptions, StorePorts,
    StoreQuery, StoreRuntime, Subscription,
};

/// Runs spawned tasks on a `LocalPool`, only when the test says so.
#[derive(Clone, Default)]
pub(super) struct Executor {
    queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>>,
}

impl Executor {
    pub fn spawner(&self) -> Arc<dyn super::Spawner> {
        let queue = self.queue.clone();
        Arc::new(move |task: BoxFuture<'static, ()>| queue.lock().push(task))
    }

    /// Polls every task until none can make progress.
    pub fn run(&self, pool: &mut LocalPool) {
        loop {
            let tasks = std::mem::take(&mut *self.queue.lock());
            for task in tasks {
                pool.spawner().spawn_local(task).expect("spawn");
            }
            pool.run_until_stalled();
            if self.queue.lock().is_empty() {
                return;
            }
        }
    }
}

/// A store over fakes plus the controls a test needs.
pub(super) struct Harness {
    pub clock: Arc<FakeClockPort>,
    pub resources: Arc<FakeResourcePort>,
    pub tables: Arc<FakeTableFeedPort>,
    pub store: ResourceStore,
    exec: Executor,
    pool: LocalPool,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_options(StoreOptions::default())
    }

    pub fn with_objects(objects: impl IntoIterator<Item = Resource>) -> Self {
        let h = Self::new();
        for o in objects {
            h.resources.insert(o);
        }
        h
    }

    pub fn with_options(options: StoreOptions) -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
        let tables = Arc::new(FakeTableFeedPort::with_clock(clock.clone()));
        let exec = Executor::default();
        let store = ResourceStore::new(
            ClusterId::new("test", &ContextName::from("kind")),
            StorePorts {
                resources: resources.clone(),
                tables: tables.clone(),
            },
            StoreRuntime {
                spawner: exec.spawner(),
                clock: clock.clone(),
            },
            options,
        );
        Self {
            clock,
            resources,
            tables,
            store,
            exec,
            pool: LocalPool::new(),
        }
    }

    /// Runs every task until idle.
    pub fn settle(&mut self) {
        self.exec.run(&mut self.pool);
    }

    /// Advances the fake clock by `secs` and settles.
    pub fn advance(&mut self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
        self.settle();
    }

    pub fn subscribe(&mut self, query: StoreQuery) -> Subscription {
        let sub = self.store.subscribe(query);
        self.settle();
        sub
    }
}

/// Default options with a 10 s grace period.
pub(super) fn options_with_grace(secs: u64) -> StoreOptions {
    StoreOptions {
        config: StoreConfig {
            idle_grace: Duration::from_secs(secs),
            ..StoreConfig::default()
        },
        ..StoreOptions::default()
    }
}

pub(super) fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

pub(super) fn widgets() -> Gvk {
    Gvk::new("example.com", "v1", "Widget")
}

pub(super) fn all(gvk: Gvk) -> StoreQuery {
    StoreQuery::new(gvk, WatchScope::Cluster)
}

pub(super) fn in_namespaces(gvk: Gvk, names: &[&str]) -> StoreQuery {
    StoreQuery::new(
        gvk,
        WatchScope::Namespaces(names.iter().map(|n| (*n).to_owned()).collect()),
    )
}

/// A pod `ns/name` at resource version `rv`.
pub(super) fn p(ns: &str, name: &str, rv: &str) -> Resource {
    let mut r = pod().namespace(ns).name(name).build();
    r.meta.resource_version = Some(rv.into());
    r
}

pub(super) fn batch(deltas: Vec<Delta<Resource>>) -> DeltaBatch<Resource> {
    DeltaBatch::from_deltas(deltas)
}

/// A watch timeline: `items` at 0 s, 1 s, 2 s, ..., then kept open.
pub(super) fn timeline(items: Vec<DeltaBatch<Resource>>) -> Timeline<DeltaBatch<Resource>> {
    items
        .into_iter()
        .enumerate()
        .fold(Timeline::new(), |t, (i, b)| {
            t.ok_at(Duration::from_secs(i as u64), b)
        })
        .keep_open()
}

/// The next ready item, if any.
pub(super) fn next(sub: &mut Subscription) -> Option<StoreDelta> {
    sub.next().now_or_never().flatten()
}

/// A consumer's copy of the rows, kept by applying every item.
#[derive(Default)]
pub(super) struct Mirror {
    pub rows: Vec<Arc<StoreObject>>,
    pub items: usize,
    pub last: Option<StoreDelta>,
}

impl Mirror {
    /// Applies every ready item; returns how many there were.
    pub fn drain(&mut self, sub: &mut Subscription) -> usize {
        let mut n = 0;
        while let Some(delta) = next(sub) {
            delta.apply_to(&mut self.rows);
            assert_eq!(self.rows.len(), delta.len, "len matches the applied rows");
            self.last = Some(delta);
            n += 1;
        }
        self.items += n;
        n
    }

    /// `ns/name` of every row, in order.
    pub fn names(&self) -> Vec<String> {
        names(&self.rows)
    }

    pub fn last_rows(&self) -> &RowChange {
        &self.last.as_ref().expect("an item").rows
    }
}

pub(super) fn names(rows: &[Arc<StoreObject>]) -> Vec<String> {
    rows.iter()
        .map(|o| format!("{}/{}", o.namespace().unwrap_or("-"), o.name()))
        .collect()
}
