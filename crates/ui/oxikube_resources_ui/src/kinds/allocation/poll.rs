//! Polling the metrics port for the usage segment while a panel is open.

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Task};
use oxikube_app::allocation::{UsageState, namespace_usage, node_usage};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::MetricsPort;
use oxikube_runtime::spawn_kube;

use crate::detail::DetailView;

/// How often an open panel asks for usage. metrics-server scrapes about once a minute, so asking
/// more often would only repeat an answer; this is the same order as `kubectl top`'s freshness.
pub const USAGE_INTERVAL: Duration = Duration::from_secs(15);

/// How soon to ask again after an absence (no metrics-server yet, a failed call): a transient
/// failure should not leave the panel saying "unavailable" for a full [`USAGE_INTERVAL`].
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

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

impl UsageTarget {
    /// The target for `subject` in the cluster `view` shows; `None` while that cluster's session
    /// has no metrics port yet.
    pub fn of(view: &DetailView, subject: UsageSubject) -> Option<Self> {
        let cluster = view.target().cluster.clone();
        let port = view.deps().sessions.get(&cluster)?.metrics()?;
        Some(Self {
            port,
            cluster,
            subject,
        })
    }
}

/// Starts polling for the entity `cx` belongs to; keep the returned task for as long as the
/// polling should run.
///
/// `target` is asked before every call (the panel may follow another object, or the session may
/// have reconnected) and may answer `None` while there is nothing to ask. `apply` receives each
/// answer; it is only called when the answer differs from the last (or is for another cluster or
/// subject), so an unchanged reading never wakes the panel, and with `Pending` when the subject
/// changes, so the old subject's reading is not shown for the new one. The call itself runs on
/// the Kubernetes runtime, never on the UI thread.
pub fn poll_usage<T: 'static>(
    cx: &mut Context<T>,
    target: impl Fn(&T, &App) -> Option<UsageTarget> + 'static,
    apply: impl Fn(&mut T, UsageState, &mut Context<T>) + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        // What the panel was last told, and for whom (cluster and subject): an answer for another
        // one is news even when it reads the same.
        let mut last: Option<((ClusterId, UsageSubject), UsageState)> = None;
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
            let who = (cluster.clone(), subject.clone());
            if last.as_ref().is_some_and(|(seen, _)| *seen != who) {
                // The panel moved to another subject: drop the old subject's reading until the
                // new one arrives.
                last = None;
                if this
                    .update(cx, |this, cx| apply(this, UsageState::Pending, cx))
                    .is_err()
                {
                    break;
                }
            }
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
            let wait = match state {
                UsageState::Unavailable(_) => RETRY_INTERVAL,
                _ => USAGE_INTERVAL,
            };
            if last.as_ref().map(|(_, seen)| seen) != Some(&state) {
                last = Some((who, state.clone()));
                if this.update(cx, |this, cx| apply(this, state, cx)).is_err() {
                    break;
                }
            }
            cx.background_executor().timer(wait).await;
        }
    })
}
