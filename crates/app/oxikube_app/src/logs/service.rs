//! [`LogService`]: opens log sessions over a `LogPort` and keeps their memory bounded.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use oxikube_domain::ids::ClusterId;
use oxikube_ports::{LogOptions, LogPort, ResourceReader};
use parking_lot::Mutex;

use super::aggregate::{
    AggShared, AggregatePorts, AggregateSession, AggregateSpec, AggregateView, Coordinator,
};
use super::bounds::{BoundCell, Bounds};
use super::churn::{Overlap, ReconnectPolicy, clamp_reconnect_retries};
use super::driver::Driver;
use super::options::{LogConfig, LogRuntime, clamp_max_streams};
use super::session::{LogReader, LogSession, Restart};
use super::shared::Shared;
use super::target::LogTarget;
use crate::store::spawn_guarded;

/// Opens and tracks log sessions. One per app: the sessions of every cluster share the runtime and
/// the `logs.buffer_lines` bound, which a cluster may override for its own sessions.
///
/// Plain async Rust over [`LogPort`]: no gpui, no kube. The UI, the MCP `get_logs` tool and the
/// tests share this one implementation.
pub struct LogService {
    runtime: LogRuntime,
    config: LogConfig,
    bounds: Mutex<Bounds>,
    max_streams: Arc<AtomicUsize>,
    /// `logs.reconnect_retries`, shared with every session's read.
    retries: Arc<AtomicU32>,
    next_id: AtomicU64,
    sessions: Mutex<Vec<Tracked>>,
}

/// An open session and the bound it reads.
struct Tracked {
    shared: Weak<Shared>,
    bound: BoundCell,
}

impl LogService {
    /// A service that runs session tasks on `runtime.spawner`. Nothing is spawned until a session
    /// is opened.
    pub fn new(runtime: LogRuntime, config: LogConfig) -> Self {
        Self {
            runtime,
            bounds: Mutex::new(Bounds::new(config.buffer_lines)),
            max_streams: Arc::new(AtomicUsize::new(clamp_max_streams(config.max_streams))),
            retries: Arc::new(AtomicU32::new(match config.reconnect {
                ReconnectPolicy::Backoff(backoff) => clamp_reconnect_retries(backoff.max_retries),
                ReconnectPolicy::Never => 0,
            })),
            config,
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(Vec::new()),
        }
    }

    /// Opens a session reading `target` over `port` with `options`, and returns at once: the
    /// stream is opened on the runtime, never on the caller. The session starts `Connecting`;
    /// a pod that does not exist or a denied `pods/log` is its `Failed` state, not an error here.
    ///
    /// The container is the target's, else `options.container`; the session's
    /// [`target`](LogReader::target) and [`options`](LogReader::options) say which won.
    pub fn open(
        &self,
        port: Arc<dyn LogPort>,
        target: LogTarget,
        options: LogOptions,
    ) -> LogSession {
        let bound = self.bounds.lock().default_cell();
        self.open_bounded(bound, port, None, target, options)
    }

    /// [`LogService::open`] for a session of `cluster`: it keeps the cluster's own
    /// `buffer_lines` ([`LogService::set_cluster_buffer_lines`]) when it has one.
    pub fn open_in(
        &self,
        cluster: &ClusterId,
        port: Arc<dyn LogPort>,
        target: LogTarget,
        options: LogOptions,
    ) -> LogSession {
        let bound = self.bounds.lock().cell_for(cluster);
        self.open_bounded(bound, port, None, target, options)
    }

    /// [`LogService::open_in`] for a session that follows its pod's life (E08-S07): it reads the
    /// pod through `ports.resources` as the stream opens and again when the stream ends, so the
    /// session says why it ended ([`EndReason::PodFinished`](super::EndReason),
    /// [`PodReplaced`](super::EndReason::PodReplaced), [`PodDeleted`](super::EndReason::PodDeleted))
    /// and reconnects when the pod still runs. What the viewer opens.
    pub fn open_following_in(
        &self,
        cluster: &ClusterId,
        ports: AggregatePorts,
        target: LogTarget,
        options: LogOptions,
    ) -> LogSession {
        let bound = self.bounds.lock().cell_for(cluster);
        self.open_bounded(bound, ports.logs, Some(ports.resources), target, options)
    }

    fn open_bounded(
        &self,
        bound: BoundCell,
        port: Arc<dyn LogPort>,
        resources: Option<Arc<dyn ResourceReader>>,
        mut target: LogTarget,
        mut options: LogOptions,
    ) -> LogSession {
        let container = target.container.take().or(options.container.take());
        target.container.clone_from(&container);
        options.container = container;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let capacity = bound.load(Ordering::Acquire);
        let shared = Arc::new(Shared::new(id, target, options, capacity));
        {
            let mut sessions = self.sessions.lock();
            sessions.retain(|s| s.shared.strong_count() > 0);
            sessions.push(Tracked {
                shared: Arc::downgrade(&shared),
                bound: bound.clone(),
            });
        }
        let driver = Driver {
            port,
            shared: shared.clone(),
            clock: self.runtime.clock.clone(),
            config: self.config.clone(),
            buffer_lines: bound,
            retries: self.retries.clone(),
            resources,
        };
        let task = spawn_guarded(
            &self.runtime.spawner,
            driver.clone().run(Overlap::default()),
        );
        let restart = Restart {
            spawner: self.runtime.spawner.clone(),
            driver,
        };
        LogSession::new(shared, task, Some(restart))
    }

    /// Opens an aggregate session reading the pods `spec` picks (a workload, a Service or a label
    /// selector) over `ports`, their lines merged by server timestamp into one buffer, and returns
    /// at once: the selector is resolved, the pods watched and the streams opened on the runtime.
    /// An object that does not exist, has no selector or cannot be read is the session's `Failed`
    /// state, not an error here. `options` apply to every pod's stream (`options.container` is
    /// ignored: the spec names the container; `timestamps` is always on, the merge needs it).
    ///
    /// The merged buffer is bounded by `logs.buffer_lines` like any session's, and at most
    /// `logs.max_streams` containers are read at once (a session that does not follow reads at
    /// most that many in all: a stream that ends frees no slot). See [`AggregateSession`].
    pub fn open_aggregate(
        &self,
        ports: AggregatePorts,
        spec: AggregateSpec,
        options: LogOptions,
    ) -> AggregateSession {
        let bound = self.bounds.lock().default_cell();
        self.open_aggregate_bounded(bound, ports, spec, options)
    }

    /// [`LogService::open_aggregate`] for a view of `cluster`: the merged buffer keeps the
    /// cluster's own `buffer_lines` ([`LogService::set_cluster_buffer_lines`]) when it has one.
    pub fn open_aggregate_in(
        &self,
        cluster: &ClusterId,
        ports: AggregatePorts,
        spec: AggregateSpec,
        options: LogOptions,
    ) -> AggregateSession {
        let bound = self.bounds.lock().cell_for(cluster);
        self.open_aggregate_bounded(bound, ports, spec, options)
    }

    fn open_aggregate_bounded(
        &self,
        bound: BoundCell,
        ports: AggregatePorts,
        spec: AggregateSpec,
        mut options: LogOptions,
    ) -> AggregateSession {
        options.container = spec.container.clone();
        options.timestamps = true;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let target = LogTarget::aggregate(&spec.namespace, spec.label());
        let capacity = bound.load(Ordering::Acquire);
        let shared = Arc::new(Shared::new(id, target, options.clone(), capacity));
        let agg = Arc::new(AggShared::new(spec.label()));
        {
            let mut sessions = self.sessions.lock();
            sessions.retain(|s| s.shared.strong_count() > 0);
            sessions.push(Tracked {
                shared: Arc::downgrade(&shared),
                bound: bound.clone(),
            });
        }
        let coordinator = Coordinator {
            ports,
            spec,
            options,
            shared: shared.clone(),
            agg: agg.clone(),
            runtime: self.runtime.clone(),
            config: self.config.clone(),
            buffer_lines: bound,
            max_streams: self.max_streams.clone(),
            retries: self.retries.clone(),
        };
        let task = spawn_guarded(&self.runtime.spawner, coordinator.run());
        AggregateSession::new(LogSession::new(shared, task, None), AggregateView::new(agg))
    }

    /// The clock the service's tasks and deadlines run on.
    pub(crate) fn clock(&self) -> Arc<dyn oxikube_ports::ClockPort> {
        self.runtime.clock.clone()
    }

    /// Streams an aggregate reads at once (`logs.max_streams`).
    pub fn max_streams(&self) -> usize {
        self.max_streams.load(Ordering::Acquire)
    }

    /// Applies a new `logs.max_streams` (clamped) to every open aggregate and to the ones opened
    /// later: a higher value starts the pods that were left out, a lower one lets running
    /// streams finish and opens no new ones until there is room.
    pub fn set_max_streams(&self, streams: usize) {
        self.max_streams
            .store(clamp_max_streams(streams), Ordering::Release);
    }

    /// Reconnects a broken stream tries in a row before its session is `Failed`
    /// (`logs.reconnect_retries`).
    pub fn reconnect_retries(&self) -> u32 {
        self.retries.load(Ordering::Acquire)
    }

    /// Applies a new `logs.reconnect_retries` (clamped) to every open session and stream: the next
    /// failure counts against it.
    pub fn set_reconnect_retries(&self, retries: u32) {
        self.retries
            .store(clamp_reconnect_retries(retries), Ordering::Release);
    }

    /// Lines a session keeps by default (`logs.buffer_lines`).
    pub fn buffer_lines(&self) -> usize {
        self.bounds.lock().default_lines()
    }

    /// Lines the sessions of `cluster` keep: its own bound, else the default.
    pub fn buffer_lines_for(&self, cluster: &ClusterId) -> usize {
        self.bounds.lock().lines_for(cluster)
    }

    /// Applies a new `logs.buffer_lines` (clamped) to every open session without a cluster
    /// override at once and to the ones opened later. A smaller bound drops the oldest lines of
    /// the sessions now.
    pub fn set_buffer_lines(&self, lines: usize) {
        self.bounds.lock().set_default(lines);
        self.apply_bounds();
    }

    /// Sets the clusters' own `buffer_lines` (`clusters.<id>.logs.buffer_lines`), replacing the
    /// previous overrides: a cluster missing from `overrides` goes back to the default. Open
    /// sessions are resized at once like [`LogService::set_buffer_lines`] does.
    pub fn set_cluster_buffer_lines(
        &self,
        overrides: impl IntoIterator<Item = (ClusterId, usize)>,
    ) {
        self.bounds.lock().set_overrides(overrides);
        self.apply_bounds();
    }

    /// Resizes every open buffer to the bound it reads (the cells are stored first, so a
    /// concurrent commit that reads the new bound and this agree).
    fn apply_bounds(&self) {
        let open: Vec<(Arc<Shared>, BoundCell)> = {
            let mut sessions = self.sessions.lock();
            sessions.retain(|s| s.shared.strong_count() > 0);
            sessions
                .iter()
                .filter_map(|s| Some((s.shared.upgrade()?, s.bound.clone())))
                .collect()
        };
        for (shared, bound) in open {
            if !shared.is_closed() {
                shared.set_capacity(bound.load(Ordering::Acquire));
            }
        }
    }

    /// Read-only views of the open sessions, oldest first.
    pub fn sessions(&self) -> Vec<LogReader> {
        self.live().into_iter().map(LogReader::new).collect()
    }

    /// The open session reading `target`, if any (the newest, when several are open).
    pub fn reader_for(&self, target: &LogTarget) -> Option<LogReader> {
        self.live()
            .into_iter()
            .rev()
            .find(|s| &s.target == target)
            .map(LogReader::new)
    }

    fn live(&self) -> Vec<Arc<Shared>> {
        let mut sessions = self.sessions.lock();
        sessions.retain(|s| s.shared.strong_count() > 0);
        sessions
            .iter()
            .filter_map(|s| s.shared.upgrade())
            .filter(|shared| !shared.is_closed())
            .collect()
    }
}

impl std::fmt::Debug for LogService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogService")
            .field("buffer_lines", &self.buffer_lines())
            .finish_non_exhaustive()
    }
}
