//! [`store_runtime`]: where the resource stores run their feed tasks, and how their throughput
//! reaches `oxikube --perf`.

use std::sync::Arc;

use gpui::App;
use oxikube_app::store::{Spawner, StoreProbe, StoreRuntime};
use oxikube_ports::ClockPort;

/// Counts every batch the stores apply as `oxikube --perf` feed deltas (the `feed_deltas` of its
/// JSONL and summary). One relaxed atomic add, and nothing at all unless `--perf` installed a
/// recorder.
struct PerfFeedProbe;

impl StoreProbe for PerfFeedProbe {
    fn feed_batch(&self, events: usize) {
        oxikube_runtime::perf::record_feed_deltas(events as u64);
    }
}

/// The [`StoreRuntime`] for the stores the tables read: feed tasks run on the Tokio bridge (the
/// runtime `spawn_kube` uses, so kube-rs watches have their reactor), or on GPUI's background
/// executor under the deterministic test runtime (no OS threads). Either way the store keeps an
/// abort-on-drop guard for every task it spawns. Applied batches are counted by `--perf`.
pub fn store_runtime(clock: Arc<dyn ClockPort>, cx: &App) -> StoreRuntime {
    let spawner: Arc<dyn Spawner> = match oxikube_runtime::handle(cx) {
        Some(handle) => Arc::new(move |task| {
            handle.spawn(task);
        }),
        None => {
            let executor = cx.background_executor().clone();
            Arc::new(move |task| executor.spawn(task).detach())
        }
    };
    StoreRuntime {
        spawner,
        clock,
        probe: Some(Arc::new(PerfFeedProbe)),
    }
}
