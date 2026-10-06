//! [`store_runtime`]: where the resource stores run their feed tasks.

use std::sync::Arc;

use gpui::App;
use oxikube_app::store::{Spawner, StoreRuntime};
use oxikube_ports::ClockPort;

/// The [`StoreRuntime`] for the stores the tables read: feed tasks run on the Tokio bridge (the
/// runtime `spawn_kube` uses, so kube-rs watches have their reactor), or on GPUI's background
/// executor under the deterministic test runtime (no OS threads). Either way the store keeps an
/// abort-on-drop guard for every task it spawns.
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
    StoreRuntime { spawner, clock }
}
