//! The task behind one running forward: it watches the target, resolves the pod, starts the
//! port's forward on the same local port each time, and feeds the state machine.
//!
//! Wakes on three things only (no polling): a watch snapshot, an error from the running
//! forward, and the backoff timer of a pending attempt. A snapshot that makes the current pod
//! unusable drops the forward and re-resolves at once; a snapshot that makes a waiting forward
//! resolvable tries at once. The task ends when the forward fails (or is aborted by a stop).

use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::future::{Either, FutureExt as _};
use futures::stream::{BoxStream, StreamExt as _};
use oxikube_domain::Resource;
use oxikube_domain::portforward::ForwardSpec;
use oxikube_domain::session::WatchScope;
use oxikube_domain::view::SERVICE_NAME_LABEL;
use oxikube_ports::{ForwardHandle, PodRef, PortForwardError, PortForwardPort};

use super::backend::ForwardBackend;
use super::model::ForwardId;
use super::registry::Inner;
use super::resolve::{TargetView, Unresolved, pod_usable, resolve};
use super::state::{ForwardEvent, ForwardState};
use crate::network::ServiceBackends;
use crate::store::{LabelSelector, LabelTerm, StoreFilter, StoreQuery};

/// Which feed a watch item comes from.
#[derive(Debug, Clone, Copy)]
enum Part {
    Pod,
    Service,
    Slices,
}

/// The merged watch of a forward's target.
pub(super) struct TargetWatch(BoxStream<'static, (Part, Result<Vec<Resource>, String>)>);

/// Watches what `spec`'s target needs: the pod, or the Service and its EndpointSlices.
pub(super) fn watch_target(
    backend: &dyn ForwardBackend,
    spec: &ForwardSpec,
) -> Option<TargetWatch> {
    let target = &spec.target;
    let cluster = &target.cluster;
    let namespace = target.namespace.as_deref()?.to_owned();
    let scope = WatchScope::Namespaces(vec![namespace]);
    let by_name = StoreFilter {
        name: Some(target.name.to_string()),
        ..StoreFilter::default()
    };
    let tag = |part: Part, watch: super::backend::ObjectWatch| watch.map(move |item| (part, item));
    if spec.targets_service() {
        let service = StoreQuery::new(target.gvk.clone(), scope.clone()).with_filter(by_name);
        let slices = StoreQuery::new(ServiceBackends::slice_gvk(), scope).with_selector(Some(
            LabelSelector::from_terms(vec![LabelTerm::Eq(
                SERVICE_NAME_LABEL.to_owned(),
                target.name.to_string(),
            )]),
        ));
        let service = tag(Part::Service, backend.watch(cluster, service)?);
        let slices = tag(Part::Slices, backend.watch(cluster, slices)?);
        Some(TargetWatch(
            futures::stream::select(service, slices).boxed(),
        ))
    } else {
        let pod = StoreQuery::new(target.gvk.clone(), scope).with_filter(by_name);
        Some(TargetWatch(
            tag(Part::Pod, backend.watch(cluster, pod)?).boxed(),
        ))
    }
}

/// One running forward. See the [module docs](self).
pub(super) struct Supervisor {
    pub id: ForwardId,
    pub generation: u64,
    pub spec: ForwardSpec,
    /// The local port to bind: the spec's, then the one first bound, kept across reconnects.
    pub local_port: Option<u16>,
    pub port: Arc<dyn PortForwardPort>,
    pub inner: Weak<Inner>,
}

enum Wake {
    Watch(Option<(Part, Result<Vec<Resource>, String>)>),
    Forward(Option<PortForwardError>),
    Timer,
}

impl Supervisor {
    pub(super) async fn run(mut self, mut watch: TargetWatch) {
        let service = self.spec.targets_service();
        let mut view = TargetView::default();
        let mut handle: Option<ForwardHandle> = None;
        // When the next attempt is due; `Some(ZERO)` tries as soon as the view is ready.
        let mut retry_in = Some(Duration::ZERO);
        loop {
            let Some(inner) = self.inner.upgrade() else {
                return;
            };
            let clock = inner.clock.clone();
            drop(inner);
            let wake = {
                let watch_next = watch.0.next().map(Wake::Watch);
                let forward_next = match handle.as_mut() {
                    Some(h) => Either::Left(h.errors.next().map(Wake::Forward)),
                    None => Either::Right(futures::future::pending()),
                };
                let timer = match retry_in.filter(|_| view.is_ready(service)) {
                    Some(delay) => Either::Left(async move {
                        clock.sleep(delay).await;
                        Wake::Timer
                    }),
                    None => Either::Right(futures::future::pending()),
                };
                futures::select_biased! {
                    w = watch_next.fuse() => w,
                    w = forward_next.fuse() => w,
                    w = timer.fuse() => w,
                }
            };
            match wake {
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
                            retry_in = Some(Duration::ZERO);
                        }
                        (_, false) if self.resolvable(&view) => {
                            let reason = "a ready pod is available".to_owned();
                            if !self.event(ForwardEvent::TargetChanged { reason }) {
                                return;
                            }
                            retry_in = Some(Duration::ZERO);
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
                        retry_in = self.after(ForwardEvent::Lost(error));
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
                    retry_in = self.after(ForwardEvent::Lost(lost));
                }
                Wake::Timer => {
                    retry_in = None;
                    match self.connect(&view).await {
                        Ok(h) => handle = Some(h),
                        Err(event) => retry_in = self.after(event),
                    }
                }
            }
            if !self.alive() {
                return;
            }
        }
    }

    /// Resolves and starts the port's forward; the error is the event to feed.
    async fn connect(&mut self, view: &TargetView) -> Result<ForwardHandle, ForwardEvent> {
        let service = self.spec.targets_service();
        let previous = self.current_pod();
        let resolved = resolve(view, service, &self.spec.remote_port, previous.as_deref())
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
        let handle = self
            .port
            .start(pod, resolved.port, self.spec.bind, self.local_port)
            .await
            .map_err(ForwardEvent::Lost)?;
        self.local_port = Some(handle.local_addr.port());
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

fn view_apply(view: &mut TargetView, part: Part, mut objects: Vec<Resource>) {
    match part {
        Part::Pod => view.pod = Some(objects.pop()),
        Part::Service => view.service = Some(objects.pop()),
        Part::Slices => view.slices = Some(objects),
    }
}
