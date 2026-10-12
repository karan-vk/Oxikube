//! The cluster overview's numbers over testkit fakes: the distribution table, the tallies
//! (including a property test against a full recompute), the bounded top lists, the feed over a
//! scripted store, and the history queries. A deterministic local executor and the fake clock:
//! no runtime, no threads, no sleeps.

mod distribution;
mod feed;
mod history;
mod tally;
mod top;

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt as _;
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline};
use parking_lot::Mutex;

use crate::store::{ResourceStore, StoreConfig, StoreOptions, StorePorts, StoreRuntime};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind"))
}

/// Runs spawned tasks on a `LocalPool`, only when the test says so.
#[derive(Clone, Default)]
pub(super) struct Executor {
    queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>>,
}

impl Executor {
    fn spawner(&self) -> Arc<dyn crate::store::Spawner> {
        let queue = self.queue.clone();
        Arc::new(move |task: BoxFuture<'static, ()>| queue.lock().push(task))
    }

    /// Polls every task until none can make progress.
    fn run(&self, pool: &mut LocalPool) {
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

/// A store over fakes, its executor and clock.
pub(super) struct Harness {
    pub clock: Arc<FakeClockPort>,
    pub resources: Arc<FakeResourcePort>,
    pub store: ResourceStore,
    exec: Executor,
    pub pool: LocalPool,
}

impl Harness {
    pub fn new() -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
        let tables = Arc::new(FakeTableFeedPort::with_clock(clock.clone()));
        let exec = Executor::default();
        let options = StoreOptions {
            config: StoreConfig {
                idle_grace: Duration::ZERO,
                ..StoreConfig::default()
            },
            ..StoreOptions::default()
        };
        let store = ResourceStore::new(
            cluster(),
            StorePorts {
                resources: resources.clone(),
                tables,
            },
            StoreRuntime {
                spawner: exec.spawner(),
                clock: clock.clone(),
                probe: None,
            },
            options,
        );
        Self {
            clock,
            resources,
            store,
            exec,
            pool: LocalPool::new(),
        }
    }

    /// Runs `task` on the harness's executor (the feed's driver).
    pub fn spawn(&self, task: BoxFuture<'static, ()>) {
        self.exec.queue.lock().push(task);
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
