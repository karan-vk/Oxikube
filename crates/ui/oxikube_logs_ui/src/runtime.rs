//! [`log_runtime`]: where `LogService` runs its stream tasks.

use std::sync::Arc;

use gpui::App;
use oxikube_app::logs::LogRuntime;
use oxikube_app::store::Spawner;
use oxikube_ports::ClockPort;

/// The [`LogRuntime`] for the app's one `LogService`: session tasks run on the Tokio bridge (the
/// runtime `spawn_kube` uses, so kube-rs streams have their reactor), or on GPUI's background
/// executor under the deterministic test runtime (no OS threads). Either way the service keeps an
/// abort-on-drop guard for every task it spawns, so dropping a session cancels its stream.
pub fn log_runtime(clock: Arc<dyn ClockPort>, cx: &App) -> LogRuntime {
    let spawner: Arc<dyn Spawner> = match oxikube_runtime::handle(cx) {
        Some(handle) => Arc::new(move |task| {
            handle.spawn(task);
        }),
        None => {
            let executor = cx.background_executor().clone();
            Arc::new(move |task| executor.spawn(task).detach())
        }
    };
    LogRuntime { spawner, clock }
}
