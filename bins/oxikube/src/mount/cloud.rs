//! Cloud discovery (E18-S05): the `oxikube_cloud` adapters (EKS through `aws`, GKE through
//! `gcloud`, AKS through `az`) behind the app's one [`CloudDiscoveryService`].
//!
//! Building it starts nothing: the CLI runner looks for binaries and probes the login shell's
//! `PATH` on its first command, and the service runs only when asked to refresh. Its refreshes
//! run on the Tokio bridge (the runtime `spawn_kube` uses, with the process driver the runner
//! needs), never on the UI thread. The managed kubeconfigs live in the sources service's
//! directory (`<config dir>/kubeconfigs`, ADR 0015) and are written through the app's `FsPort`.

use std::sync::Arc;

use gpui::App;
use oxikube_app::cloud::DiscoveryDeps;
use oxikube_app::{CloudDiscoveryService, ClusterSessionManager, KubeconfigSourcesService};
use oxikube_cloud::aws::{AwsDiscovery, AwsSettings};
use oxikube_cloud::az::AzureAksAdapter;
use oxikube_cloud::gcloud::GcloudDiscovery;
use oxikube_cloud::process::{CliRunner, ProcessRunner};
use oxikube_ports::CloudDiscoveryPort;

use crate::app_state::AppPorts;

/// The service over the three CLI adapters, sharing one runner (one process cap and one
/// login-shell probe for all of them). Settings (E18-S07) narrow it later; until then every
/// provider uses the CLI's own configuration.
pub fn discovery(
    ports: &AppPorts,
    sources: KubeconfigSourcesService,
    sessions: ClusterSessionManager,
    cx: &App,
) -> CloudDiscoveryService {
    let runner: Arc<dyn ProcessRunner> = Arc::new(CliRunner::default());
    let providers: Vec<Arc<dyn CloudDiscoveryPort>> = vec![
        Arc::new(AwsDiscovery::new(runner.clone(), AwsSettings::default())),
        Arc::new(GcloudDiscovery::new(runner.clone())),
        Arc::new(AzureAksAdapter::new(runner)),
    ];
    CloudDiscoveryService::new(DiscoveryDeps {
        providers,
        state: ports.state.clone(),
        fs: ports.clusters.fs.clone(),
        sources,
        source: ports.clusters.source.clone(),
        sessions,
        clock: ports.clusters.clock.clone(),
        spawner: crate::metrics_provider::spawner(cx),
    })
}
