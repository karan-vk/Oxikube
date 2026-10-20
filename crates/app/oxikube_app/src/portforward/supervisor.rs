//! The task behind one running forward: it watches the target, resolves the pod, starts the
//! port's forward on the same local port each time, and feeds the state machine.
//!
//! Wakes on three things only (no polling): a watch snapshot, an error from the running
//! forward, and the backoff timer of a pending attempt. A snapshot that makes the current pod
//! unusable drops the forward and re-resolves at once; a snapshot that makes a waiting forward
//! resolvable tries at once. A pending backoff is a deadline: snapshots neither cut it short nor
//! push it back. The task ends when the forward fails, or on the user's stop: then the running
//! handle is shut down gracefully (no new connections, open ones drain within the adapter's
//! grace period) before the task returns. A restart, or a task that outlives
//! [`STOP_GRACE`], is aborted instead.

use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{Either, FutureExt as _};
use futures::stream::StreamExt as _;
use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::portforward::{ForwardSpec, PortRange};
use oxikube_ports::{ClockPort, ForwardHandle, PodRef, PortForwardError, PortForwardPort};

use super::alloc::start_in_range;
use super::model::ForwardId;
use super::registry::Inner;
use super::resolve::{TargetView, Unresolved, pod_usable, resolve};
use super::state::{ForwardEvent, ForwardState};
use super::target_watch::{Part, TargetWatch, view_apply};

/// One running forward. See the [module docs](self).
pub(super) struct Supervisor {
    pub id: ForwardId,
    pub generation: u64,
    pub spec: ForwardSpec,
    /// The local port to bind: the spec's, then the one first bound, kept across reconnects.
    pub local_port: Option<u16>,
    /// Where to look for a free port when `local_port` is unset (`portforward.port_range`).
    pub port_range: Option<PortRange>,
    pub port: Arc<dyn PortForwardPort>,
    pub inner: Weak<Inner>,
    /// The pod of the last connection, preferred when a Service forward re-resolves.
    pub last_pod: Option<String>,
}

/// How long a stopping supervisor may take to drain its connections before it is aborted (the
/// adapter's own grace is 2 s).
pub(super) const STOP_GRACE: Duration = Duration::from_secs(3);

enum Wake {
    /// The user stopped or removed the forward (or its entry went away).
    Stop,
    Watch(Option<(Part, Result<Vec<Resource>, String>)>),
    Forward(Option<PortForwardError>),
    Timer,
}

impl Supervisor {
    pub(super) async fn run(mut self, mut watch: TargetWatch, mut stop: oneshot::Receiver<()>) {
        let service = self.spec.targets_service();
        let mut view = TargetView::default();
        let mut handle: Option<ForwardHandle> = None;
        // When the next attempt is due (tried once the view is ready too); `None` waits on the
        // watch for the target to become resolvable.
        let Some(clock) = self.inner.upgrade().map(|inner| inner.clock.clone()) else {
            return;
        };
        let mut retry_at = due(&*clock, Some(Duration::ZERO));
        loop {
            if self.inner.strong_count() == 0 {
                return;
            }
            let wake = {
                let watch_next = watch.0.next().map(Wake::Watch);
                let forward_next = match handle.as_mut() {
                    Some(h) => Either::Left(h.errors.next().map(Wake::Forward)),
                    None => Either::Right(futures::future::pending()),
                };
                let clock: &dyn ClockPort = &*clock;
                let timer = match retry_at.filter(|_| view.is_ready(service)) {
                    Some(at) => Either::Left(async move {
                        clock.sleep_until(at).await;
                        Wake::Timer
                    }),
                    None => Either::Right(futures::future::pending()),
                };
                futures::select_biased! {
                    _ = stop => Wake::Stop,
                    w = watch_next.fuse() => w,
                    w = forward_next.fuse() => w,
                    w = timer.fuse() => w,
                }
            };
            match wake {
                Wake::Stop => {
                    if let Some(handle) = handle.take() {
                        handle.shutdown().await;
                    }
                    return;
                }
                Wake::Watch(Some((part, Ok(objects)))) => {
                    view_apply(&mut view, part, objects);
                    if !view.is_ready(service) {
                        continue;
                    }
                    let current = self.current_pod();
                    match (&current, handle.is_some()) {
                        (Some(pod), true) if !self.still_serves(&view, pod) => {
                            handle = None;
                            self.set_handle(None);
                            let reason = format!("pod {pod} is gone or not ready");
                            if !self.event(ForwardEvent::TargetChanged { reason }) {
                                return;
                            }
                            retry_at = due(&*clock, Some(Duration::ZERO));
                        }
                        // Only when waiting on the watch: a pending backoff keeps its deadline.
                        (_, false) if retry_at.is_none() && self.resolvable(&view) => {
                            let reason = "a ready pod is available".to_owned();
                            if !self.event(ForwardEvent::TargetChanged { reason }) {
                                return;
                            }
                            retry_at = due(&*clock, Some(Duration::ZERO));
                        }
                        _ => {}
                    }
                }
                Wake::Watch(Some((_, Err(reason)))) => {
                    self.event(ForwardEvent::Unresolvable {
                        reason,
                        permanent: true,
                    });
                    return;
                }
                // The store went away: the cluster disconnected.
                Wake::Watch(None) => {
                    self.event(ForwardEvent::Unresolvable {
                        reason: "the cluster disconnected".to_owned(),
                        permanent: true,
                    });
                    return;
                }
                Wake::Forward(Some(error)) => match error {
                    PortForwardError::PodGone(_) => {
                        handle = None;
                        self.set_handle(None);
                        retry_at = due(&*clock, self.after(ForwardEvent::Lost(error)));
                    }
                    PortForwardError::Forbidden(_) => {
                        self.event(ForwardEvent::Lost(error));
                        return;
                    }
                    // One connection failed; the listener keeps serving the next ones.
                    other => self.note_connection_error(&other),
                },
                Wake::Forward(None) => {
                    handle = None;
                    self.set_handle(None);
                    let lost = PortForwardError::Disconnected("the forward ended".to_owned());
                    retry_at = due(&*clock, self.after(ForwardEvent::Lost(lost)));
                }
                Wake::Timer => {
                    retry_at = None;
                    match self.connect(&view, &*clock).await {
                        Ok(h) => handle = Some(h),
                        // No ready pod yet: wait on the watch, which retries once it resolves.
                        Err(event @ ForwardEvent::Unresolvable { .. }) => {
                            self.event(event);
                        }
                        Err(event) => retry_at = due(&*clock, self.after(event)),
                    }
                }
            }
            if !self.alive() {
                return;
            }
        }
    }

    /// Resolves and starts the port's forward; the error is the event to feed.
    async fn connect(
        &mut self,
        view: &TargetView,
        clock: &dyn ClockPort,
    ) -> Result<ForwardHandle, ForwardEvent> {
        let service = self.spec.targets_service();
        let resolved = resolve(
            view,
            service,
            &self.spec.remote_port,
            self.last_pod.as_deref(),
        )
        .map_err(|unresolved| match unresolved {
            Unresolved::Waiting(reason) => ForwardEvent::Unresolvable {
                reason,
                permanent: false,
            },
            Unresolved::Invalid(reason) => ForwardEvent::Unresolvable {
                reason,
                permanent: true,
            },
        })?;
        let namespace = self.spec.target.namespace.as_deref().unwrap_or_default();
        let pod = PodRef::new(namespace, resolved.pod.as_str());
        let started = match (self.local_port, self.port_range) {
            (None, Some(range)) => {
                let offset = (self.id.0 as u32)
                    .wrapping_mul(7919)
                    .wrapping_add(clock.now().subsec_nanosecond().unsigned_abs());
                start_in_range(
                    &*self.port,
                    &pod,
                    resolved.port,
                    self.spec.bind,
                    range,
                    offset,
                )
                .await
            }
            (local_port, _) => {
                self.port
                    .start(pod, resolved.port, self.spec.bind, local_port)
                    .await
            }
        };
        let handle = started.map_err(ForwardEvent::Lost)?;
        self.local_port = Some(handle.local_addr.port());
        self.last_pod = Some(resolved.pod.clone());
        let local_addr = handle.local_addr;
        self.set_handle(Some(&handle));
        self.event(ForwardEvent::Connected {
            pod: resolved.pod,
            local_addr,
        });
        Ok(handle)
    }

    /// Feeds `event`; returns the delay of the next attempt when the forward is reconnecting.
    fn after(&self, event: ForwardEvent) -> Option<Duration> {
        self.event(event);
        match self.state()? {
            ForwardState::Reconnecting { delay, .. } => Some(delay),
            _ => None,
        }
    }

    /// Feeds `event` to the state machine; `false` when this supervisor is no longer current.
    fn event(&self, event: ForwardEvent) -> bool {
        self.inner
            .upgrade()
            .is_some_and(|inner| inner.transition(self.id, Some(self.generation), event, |_| {}))
    }

    fn state(&self) -> Option<ForwardState> {
        self.inner.upgrade()?.state(self.id, self.generation)
    }

    /// Whether the forward still wants this supervisor (not failed, stopped or replaced).
    fn alive(&self) -> bool {
        self.state().is_some_and(|s| s.is_running())
    }

    fn current_pod(&self) -> Option<String> {
        match self.state()? {
            ForwardState::Active { pod, .. } => Some(pod),
            _ => None,
        }
    }

    fn set_handle(&self, handle: Option<&ForwardHandle>) {
        if let Some(inner) = self.inner.upgrade() {
            let stats = handle.map(|h| h.stats.clone());
            inner.edit(self.id, self.generation, |e| e.set_stats(stats));
        }
    }

    fn note_connection_error(&self, error: &PortForwardError) {
        if let Some(inner) = self.inner.upgrade() {
            let message = error.to_string();
            inner.edit(self.id, self.generation, |e| {
                e.set_last_error(Some(message))
            });
        }
    }

    /// Whether `pod` can still serve the forward in `view`.
    fn still_serves(&self, view: &TargetView, pod: &str) -> bool {
        if self.spec.targets_service() {
            resolve(view, true, &self.spec.remote_port, Some(pod)).is_ok_and(|r| r.pod == pod)
        } else {
            matches!(&view.pod, Some(Some(p)) if pod_usable(p) && &*p.meta.name == pod)
        }
    }

    fn resolvable(&self, view: &TargetView) -> bool {
        resolve(
            view,
            self.spec.targets_service(),
            &self.spec.remote_port,
            None,
        )
        .is_ok()
    }
}

/// The deadline `delay` from now on `clock`.
fn due(clock: &dyn ClockPort, delay: Option<Duration>) -> Option<Timestamp> {
    let now = clock.now();
    delay.map(|delay| now.saturating_add(delay).unwrap_or(now))
}
