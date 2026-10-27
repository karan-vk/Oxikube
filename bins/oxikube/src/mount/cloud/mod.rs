//! Cloud discovery (E18-S05): the `oxikube_cloud` adapters (EKS through `aws`, GKE through
//! `gcloud`, AKS through `az`) behind the app's one [`CloudDiscoveryService`].
//!
//! Building it starts nothing: the CLI runner looks for binaries and probes the login shell's
//! `PATH` on its first command, and the service runs only when asked to refresh. Its refreshes
//! run on the Tokio bridge (the runtime `spawn_kube` uses, with the process driver the runner
//! needs), never on the UI thread. The managed kubeconfigs live in the sources service's
//! directory (`<config dir>/kubeconfigs`, ADR 0015) and are written through the app's `FsPort`.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use gpui::App;
use oxikube_app::cloud::{DiscoveryConfig, DiscoveryDeps};
use oxikube_app::{CloudDiscoveryService, ClusterSessionManager, KubeconfigSourcesService};
use oxikube_cloud::aws::{AwsDiscovery, AwsSettings};
use oxikube_cloud::az::AzureAksAdapter;
use oxikube_cloud::gcloud::GcloudDiscovery;
use oxikube_cloud::process::{Cli, CliRunner, CliRunnerConfig};
use oxikube_ports::CloudProvider;
use oxikube_settings::{CloudSettings, Settings as _};

use crate::app_state::AppPorts;

#[cfg(test)]
mod tests;

/// The service over the three CLI adapters, sharing one runner (one process cap and one
/// login-shell probe for all of them), configured from the `cloud` settings (E18-S07) now and on
/// every change: CLI paths, parallelism and timeout reach the runner, the scan lists reach the
/// adapters, the TTL and the enabled providers reach the service. The next run uses them; no
/// restart.
pub fn discovery(
    ports: &AppPorts,
    sources: KubeconfigSourcesService,
    sessions: ClusterSessionManager,
    cx: &mut App,
) -> CloudDiscoveryService {
    install(
        cx,
        DiscoveryDeps {
            providers: Vec::new(),
            state: ports.state.clone(),
            fs: ports.clusters.fs.clone(),
            sources,
            source: ports.clusters.source.clone(),
            sessions,
            clock: ports.clusters.clock.clone(),
            spawner: crate::metrics_provider::spawner(cx),
        },
    )
}

/// [`discovery`] over `deps`, whose `providers` it replaces with the three CLI adapters.
fn install(cx: &mut App, mut deps: DiscoveryDeps) -> CloudDiscoveryService {
    let settings = CloudSettings::resolve(cx);
    let runner = Arc::new(CliRunner::new(runner_config(&settings)));
    let aws = Arc::new(AwsDiscovery::new(runner.clone(), aws_settings(&settings)));
    let gcloud = Arc::new(GcloudDiscovery::new(runner.clone()));
    let az = Arc::new(AzureAksAdapter::new(runner.clone()));
    deps.providers = vec![aws.clone(), gcloud.clone(), az.clone()];
    let service = CloudDiscoveryService::new(deps);
    let adapters = Adapters {
        runner,
        aws,
        gcloud,
        az,
    };
    adapters.apply(&settings, &service);
    let followed = service.clone();
    CloudSettings::observe(cx, move |cx| {
        adapters.apply(&CloudSettings::resolve(cx), &followed);
    })
    .detach();
    service
}

/// What the settings configure besides the service itself.
struct Adapters {
    runner: Arc<CliRunner>,
    aws: Arc<AwsDiscovery>,
    gcloud: Arc<GcloudDiscovery>,
    az: Arc<AzureAksAdapter>,
}

impl Adapters {
    fn apply(&self, settings: &CloudSettings, service: &CloudDiscoveryService) {
        self.runner.reconfigure(runner_config(settings));
        self.aws.set_settings(aws_settings(settings));
        self.gcloud.set_projects(settings.gcloud_projects.clone());
        self.az.set_subscriptions(settings.az_subscriptions.clone());
        service.set_config(service_config(settings));
    }
}

/// The CLI runner's configuration: path overrides, default timeout and process cap.
fn runner_config(settings: &CloudSettings) -> CliRunnerConfig {
    let overrides = [
        (Cli::Aws, &settings.aws_path),
        (Cli::Gcloud, &settings.gcloud_path),
        (Cli::Az, &settings.az_path),
        (
            Cli::GkeGcloudAuthPlugin,
            &settings.gke_gcloud_auth_plugin_path,
        ),
        (Cli::Kubelogin, &settings.kubelogin_path),
    ];
    CliRunnerConfig {
        paths: overrides
            .into_iter()
            .filter_map(|(cli, path)| Some((cli, path.clone()?)))
            .collect::<BTreeMap<_, _>>(),
        default_timeout: settings.command_timeout,
        max_concurrent: settings.parallelism,
        ..CliRunnerConfig::default()
    }
}

fn aws_settings(settings: &CloudSettings) -> AwsSettings {
    AwsSettings {
        profiles: settings.aws_profiles.clone(),
        regions: settings.aws_regions.clone(),
    }
}

/// The service's configuration: the TTL, the scope fan-out and the providers switched off.
fn service_config(settings: &CloudSettings) -> DiscoveryConfig {
    DiscoveryConfig {
        ttl: settings.cache_ttl,
        max_parallel_scopes: NonZeroUsize::new(settings.parallelism)
            .unwrap_or(DiscoveryConfig::DEFAULT_MAX_PARALLEL_SCOPES),
        disabled: CloudProvider::ALL
            .into_iter()
            .filter(|provider| !settings.is_enabled(*provider))
            .collect(),
    }
}
