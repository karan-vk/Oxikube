//! History metrics in the running app (E13-S05): the glue between the metrics provider service
//! (`oxikube_app::metrics_provider`), the PromQL adapter (`oxikube_prometheus`) and the settings.
//!
//! * [`build`] makes the app's one [`MetricsProviderService`] and the per-cluster PromQL port. The
//!   port reaches an in-cluster Prometheus through the clusters' service proxies
//!   ([`SessionProxies`]) and is told where each cluster's provider is by the service
//!   ([`PromqlSink`]). Constructing spawns and reads nothing.
//! * [`start`] runs on the first window mount: a foreground task follows the session updates
//!   (it waits, and per update does a short in-memory evaluation, so an idle app has no metrics
//!   task on the Tokio runtime), and every change of the settings is passed on
//!   ([`follow_settings`]), so an edit of `prometheus` in `settings.json` (or the page's switch)
//!   applies at once without a reconnect. Only a run of detection goes to the Tokio bridge: when a
//!   cluster connects and its settings ask for one. Nothing is probed before that, and a cluster
//!   whose settings turn metrics off is never probed.
//! * [`SettingsMetricsWriter`] is how `cluster::ToggleMetrics` reaches `settings.json`: it applies
//!   each edit on the foreground with the comment-preserving settings editor, and the edit flows
//!   back through [`follow_settings`].
//!
//! The window's side (the page, the sidebar entry) is `mount::metrics`.

mod writer;

use std::sync::Arc;

use gpui::{App, Subscription, Task};
use oxikube_app::metrics_provider::Resolved;
use oxikube_app::store::Spawner;
use oxikube_app::{ClusterSessionManager, EndpointSink, MetricsProviderService};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{PromqlPort, ProviderDetectorPort, ServiceProxyPort};
use oxikube_prometheus::{PrometheusPort, PromqlClient, ProviderDetector, ServiceProxies};
use oxikube_settings::{ClusterSettings, Settings as _};

pub use writer::SettingsMetricsWriter;

use crate::app_state::AppPorts;

/// The metrics side of the app's services.
#[derive(Clone)]
pub struct MetricsServices {
    /// Which Prometheus-compatible service each cluster's history comes from.
    pub provider: MetricsProviderService,
    /// PromQL queries per cluster (the graphs read through this).
    pub promql: Arc<dyn PromqlPort>,
}

/// The services over `ports`, following `sessions`. Constructs only.
pub fn build(ports: &AppPorts, sessions: &ClusterSessionManager) -> MetricsServices {
    let client = Arc::new(match &ports.secrets {
        Some(secrets) => PromqlClient::with_secret_store(secrets.clone()),
        None => PromqlClient::new(),
    });
    let port = Arc::new(
        PrometheusPort::new(client.clone())
            .with_proxies(Arc::new(SessionProxies(sessions.clone()))),
    );
    let detector: Arc<dyn ProviderDetectorPort> = Arc::new(ProviderDetector::with_client(client));
    let sink = Arc::new(PromqlSink(port.clone()));
    MetricsServices {
        provider: MetricsProviderService::new(sessions.clone(), detector, sink),
        promql: port,
    }
}

/// Finds a connected cluster's service proxy for the PromQL port.
struct SessionProxies(ClusterSessionManager);

impl ServiceProxies for SessionProxies {
    fn service_proxy(&self, cluster: &ClusterId) -> Option<Arc<dyn ServiceProxyPort>> {
        self.0.get(cluster)?.service_proxy()
    }
}

/// Points the PromQL port at the provider the service found.
struct PromqlSink(Arc<PrometheusPort>);

impl EndpointSink for PromqlSink {
    fn publish(&self, cluster: &ClusterId, resolved: Option<&Resolved>) {
        match resolved {
            Some(resolved) => self
                .0
                .set_endpoint(cluster.clone(), resolved.endpoint.clone()),
            None => self.0.clear_endpoint(cluster),
        }
    }
}

/// A spawner onto the Tokio bridge (GPUI's background executor under the deterministic test
/// runtime), the one the resource stores use.
fn spawner(cx: &App) -> Arc<dyn Spawner> {
    match oxikube_runtime::handle(cx) {
        Some(handle) => Arc::new(move |task| {
            handle.spawn(task);
        }),
        None => {
            let executor = cx.background_executor().clone();
            Arc::new(move |task| executor.spawn(task).detach())
        }
    }
}

/// What [`start`] keeps running: the task that follows the sessions and the subscription to the
/// settings. Dropping it stops both.
pub struct MetricsFollow {
    _sessions: Task<()>,
    _settings: Subscription,
}

/// Starts `services`: the service follows the sessions and every settings change reaches it.
///
/// The session updates are followed by a foreground task (`MetricsProviderService::follow`: it
/// waits, and per update does a short in-memory evaluation), so an app with no connected cluster
/// has no task on the Tokio runtime for metrics (the startup scenario fails on an idle one). Only
/// a run of detection or of a check goes to the Tokio bridge, when a cluster connects and its
/// settings ask for one. Keep the result for the life of the app (or the window).
pub fn start(
    services: &MetricsServices,
    sessions: &ClusterSessionManager,
    cx: &mut App,
) -> MetricsFollow {
    // Subscribe first, then look at the clusters connected already.
    let follow = services.provider.follow();
    services.provider.attach(spawner(cx));
    let task = cx.spawn(async move |_| follow.await);
    MetricsFollow {
        _sessions: task,
        _settings: follow_settings(&services.provider, sessions, cx),
    }
}

/// Passes each change of the cluster settings to `service`. The prefs are pushed into `sessions`
/// first (an idempotent hash swap), so the service never reads a table older than the settings
/// whatever order the app's observers run in.
pub fn follow_settings(
    service: &MetricsProviderService,
    sessions: &ClusterSessionManager,
    cx: &mut App,
) -> Subscription {
    let (service, sessions) = (service.clone(), sessions.clone());
    let apply = move |cx: &App| {
        sessions.set_prefs_table(ClusterSettings::table(cx));
        service.settings_changed();
    };
    apply(cx);
    ClusterSettings::observe(cx, move |cx| apply(cx))
}

#[cfg(test)]
mod tests;
