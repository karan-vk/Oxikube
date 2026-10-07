//! The cluster adapters of the running app: `oxikube_kube` behind the ports the app layer takes.
//!
//! | File | Holds |
//! |---|---|
//! | `sources` | [`LazyKubeSources`]: the kubeconfig catalog (`ClusterSourcePort`), built on first use |
//! | `connector` | [`SourcesConnector`]: the kube connector (`ClusterConnectorPort`), synced with the catalog, giving each connection its `DescribePort` (`oxikube_describe`) |
//! | `budget` | [`WatchBudgets`]: each connection's watch budget from the `watch_budget` setting, behind the resource stores' budget hook, with its counters (E04-F543) |
//! | `clock` | [`SystemClock`]: the wall clock and Tokio's timer (`ClockPort`) |
//! | `secrets` | [`MemorySecrets`]: an in-process `SecretStorePort` for the source adapter |
//!
//! [`kube_adapters`] builds the bundle at start-up (the `StateDb` stage of the init order).
//! Building reads nothing and opens no socket: the catalog is read by its first use, after the
//! first frame, and a cluster is contacted when the user connects it. Per-connection ports
//! (discovery, resources, Table feeds, health, logs, exec, metrics, access review) come from the
//! connector, one bundle per connection, through `oxikube_app::ClusterSessionManager`.

mod budget;
mod clock;
mod connector;
mod secrets;
mod sources;

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_describe::DescribePreference;
use oxikube_ports::UserSource;
use oxikube_runtime::StdFs;
use tokio::runtime::Handle;

pub use budget::{WatchBudgets, report_line};
pub use clock::SystemClock;
pub use connector::SourcesConnector;
pub use secrets::MemorySecrets;
pub use sources::LazyKubeSources;

use crate::app_state::ClusterAdapters;

/// The app's cluster adapters: the kubeconfig catalog over `initial` (the user's source list from
/// settings), the kube connector over it, the wall clock on `runtime`, the local file system, and
/// `kubeconfigs_dir` for pasted kubeconfigs. Constructs only; see the [module docs](self).
pub fn kube_adapters(
    initial: Vec<UserSource>,
    runtime: Handle,
    kubeconfigs_dir: PathBuf,
) -> ClusterAdapters {
    let sources = Arc::new(LazyKubeSources::new(
        initial,
        Arc::new(MemorySecrets::default()),
        true,
    ));
    let describe = DescribePreference::default();
    let connector = SourcesConnector::new(sources.clone(), describe.clone());
    let budgets = WatchBudgets::new(connector.kube());
    ClusterAdapters {
        connector: Arc::new(connector),
        budgets,
        describe,
        source: sources,
        clock: Arc::new(SystemClock::new(runtime)),
        fs: Arc::new(StdFs),
        kubeconfigs_dir,
    }
}
