//! [`SettingsMetricsWriter`]: `cluster::ToggleMetrics`' way into `settings.json`.
//!
//! The command handler in `oxikube_app` runs on whatever task dispatched the command and knows
//! nothing of GPUI; the settings store is a GPUI global that only the foreground thread may touch.
//! The writer is the hand-over: [`MetricsSettingsWriter::set_enabled`] queues a request on a
//! channel and awaits its reply, and one foreground task (owned by the writer, dropped with it)
//! applies each request with [`ClusterSettings::set_metrics_enabled`], the comment-preserving
//! editor. Requests are applied in order, one at a time, so two quick toggles never race on the
//! file.

use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use gpui::{App, Task};
use oxikube_app::metrics_provider::MetricsSettingsWriter;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_settings::ClusterSettings;

struct Request {
    cluster: ClusterId,
    name_hint: Option<String>,
    enabled: bool,
    reply: oneshot::Sender<OxiResult<()>>,
}

/// Writes `prometheus.enabled` to the user's `settings.json` through the settings store.
///
/// Keep it alive for as long as commands may run (it owns the foreground task that applies the
/// edits); hand an `Arc` of it to `oxikube_app::metrics_provider::register_commands`.
pub struct SettingsMetricsWriter {
    requests: mpsc::UnboundedSender<Request>,
    _worker: Task<()>,
}

impl SettingsMetricsWriter {
    /// A writer applying edits on `cx`'s foreground executor.
    pub fn new(cx: &mut App) -> Self {
        let (requests, mut inbox) = mpsc::unbounded::<Request>();
        let worker = cx.spawn(async move |cx| {
            while let Some(request) = inbox.next().await {
                let Request {
                    cluster,
                    name_hint,
                    enabled,
                    reply,
                } = request;
                let edit = cx.update(|cx| {
                    ClusterSettings::set_metrics_enabled(
                        cx,
                        &cluster,
                        name_hint.as_deref(),
                        enabled,
                    )
                });
                // The caller may have gone away (its dispatch was dropped): the edit still ran.
                let _ = reply.send(edit.await);
            }
        });
        Self {
            requests,
            _worker: worker,
        }
    }
}

impl MetricsSettingsWriter for SettingsMetricsWriter {
    fn set_enabled(
        &self,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        enabled: bool,
    ) -> BoxFuture<'static, OxiResult<()>> {
        let (reply, answer) = oneshot::channel();
        let sent = self.requests.unbounded_send(Request {
            cluster: cluster.clone(),
            name_hint: name_hint.map(str::to_owned),
            enabled,
            reply,
        });
        async move {
            sent.map_err(|_| OxiError::internal("the settings writer has shut down"))?;
            answer
                .await
                .map_err(|_| OxiError::internal("the settings writer stopped before answering"))?
        }
        .boxed()
    }
}
