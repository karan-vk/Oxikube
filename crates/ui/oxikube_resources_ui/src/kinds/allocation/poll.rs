//! Polling the metrics port for the usage segment while a panel is open.

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Task};
use oxikube_app::allocation::{UsageState, namespace_usage, node_usage};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::MetricsPort;
use oxikube_runtime::spawn_kube;

/// How often an open panel asks for usage. metrics-server scrapes about once a minute, so asking
/// more often would only repeat an answer; this is the same order as `kubectl top`'s freshness.
pub const USAGE_INTERVAL: Duration = Duration::from_secs(15);

/// How long to wait for the session before asking again (the panel opened before the connection
/// was ready, or a reconnect is under way).
const SESSION_WAIT: Duration = Duration::from_millis(250);

/// Whose usage to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageSubject {
    /// A node, by name.
    Node(Arc<str>),
    /// The pods of a namespace, summed.
    Namespace(Arc<str>),
}

/// What to ask and of whom.
pub struct UsageTarget {
    /// The cluster's metrics port.
    pub port: Arc<dyn MetricsPort>,
    /// The cluster.
    pub cluster: ClusterId,
    /// Whose usage.
    pub subject: UsageSubject,
}

/// Starts polling for the entity `cx` belongs to; keep the returned task for as long as the
/// polling should run.
///
/// `target` is asked before every call (the panel may follow another object, or the session may
/// have reconnected) and may answer `None` while there is nothing to ask. `apply` receives each
/// answer; it is only called when the answer differs from the last, so an unchanged reading never
/// wakes the panel. The call itself runs on the Kubernetes runtime, never on the UI thread.
pub fn poll_usage<T: 'static>(
    cx: &mut Context<T>,
    target: impl Fn(&T, &App) -> Option<UsageTarget> + 'static,
    apply: impl Fn(&mut T, UsageState, &mut Context<T>) + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut last: Option<UsageState> = None;
        loop {
            let Ok(next) = this.read_with(cx, |this, cx| target(this, cx)) else {
                break;
            };
            let Some(UsageTarget {
                port,
                cluster,
                subject,
            }) = next
            else {
                cx.background_executor().timer(SESSION_WAIT).await;
                continue;
            };
            let call = spawn_kube(&*cx, async move {
                match subject {
                    UsageSubject::Node(name) => node_usage(&*port, &cluster, &name).await,
                    UsageSubject::Namespace(name) => namespace_usage(&*port, &cluster, &name).await,
                }
            });
            let state = match call.await {
                Ok(state) => state,
                Err(error) => {
                    tracing::warn!(%error, "reading metrics for an allocation view failed");
                    UsageState::Unavailable("metrics unavailable: the request failed".into())
                }
            };
            if last.as_ref() != Some(&state) {
                last = Some(state.clone());
                let alive = this.update(cx, |this, cx| apply(this, state, cx)).is_ok();
                if !alive {
                    break;
                }
            }
            cx.background_executor().timer(USAGE_INTERVAL).await;
        }
    })
}
