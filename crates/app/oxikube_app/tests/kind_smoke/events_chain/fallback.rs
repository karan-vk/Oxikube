//! Scenarios 1 and 4: a failing image pull's `Warning` reaches the `EventService` within a second
//! of a plain `kubectl` watch seeing it, and a cluster without `events.k8s.io` gives the same
//! normalised events through `core/v1`.
//!
//! The shared kind cluster serves `events.k8s.io/v1`, and turning the API group off on its API
//! server (`--runtime-config=events.k8s.io/v1=false`) would break every other suite. So the
//! "disabled" connection reads through a client that sends every `events.k8s.io` request to a
//! group no server serves (the rewrite `metrics_absent` uses for `metrics.k8s.io`), with legacy
//! discovery: the group's version list is the API server's own 404, discovery leaves the group
//! out, exactly as for a server with the group disabled, and the adapter's default choice
//! (`Auto`) falls back to `core/v1` from discovery, never from a failed watch. The reference is a
//! second, untouched connection of the same cluster, which reads `events.k8s.io/v1`. Both feeds
//! are the app's cluster-wide feed; the comparison is limited to the test's namespace.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use http::Request;
use kube::client::{Body, ClientBuilder};
use kube::config::KubeConfigOptions;
use kube::{Client, Config};
use oxikube_domain::event::{Event, EventType};
use oxikube_kube::{DiscoveryConfig, KubeDiscovery, KubeEvents, KubeResources};
use oxikube_ports::EventApi;
use oxikube_testkit::EventsConnector;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::events::{EventArrivals, PULL_FAILED_REASON, unpullable_pod};
use tower::{Layer, Service};

use super::{Chain, DEADLINE, LIVE_WITHIN, create_in, kube_connector, until};
use crate::cluster::Kind;

/// Sends `events.k8s.io` requests to a group no API server serves; everything else unchanged.
#[derive(Clone)]
struct HideEventsGroup<S> {
    inner: S,
}

struct HideEventsGroupLayer;

impl<S> Layer<S> for HideEventsGroupLayer {
    type Service = HideEventsGroup<S>;
    fn layer(&self, inner: S) -> Self::Service {
        HideEventsGroup { inner }
    }
}

impl<S: Service<Request<Body>>> Service<Request<Body>> for HideEventsGroup<S> {
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let rewritten = req
            .uri()
            .to_string()
            .replace("/apis/events.k8s.io/", "/apis/events.absent.oxikube.test/");
        *req.uri_mut() = rewritten.parse().expect("a rewritten uri");
        self.inner.call(req)
    }
}

/// A client of the kind cluster that cannot see `events.k8s.io`.
async fn client_without_events_group(kind: &Kind) -> Client {
    let options = KubeConfigOptions {
        context: Some(kind.context.to_string()),
        ..KubeConfigOptions::default()
    };
    let config = Config::from_custom_kubeconfig(kind.kubeconfig.clone(), &options)
        .await
        .expect("the kind client config");
    ClientBuilder::try_from(config)
        .expect("a client builder")
        .with_layer(&HideEventsGroupLayer)
        .build()
}

/// The service's events about the pod `name`, by uid: what the two feeds must agree on.
fn about(chain: &Chain, ns: &TestNamespace, name: &str) -> BTreeMap<String, Event> {
    chain
        .in_namespace(ns)
        .iter()
        .filter(|e| &*e.regarding.name == name && &*e.regarding.gvk.kind == "Pod")
        .map(|e| (e.uid.as_deref().unwrap_or("-").to_owned(), (**e).clone()))
        .collect()
}

/// How long after `arrivals` saw its first pull-failure `Warning` about `pod` the chain held
/// that same event (by uid).
async fn latency(
    chain: &Chain,
    ns: &TestNamespace,
    arrivals: &EventArrivals,
    pod: &str,
) -> Duration {
    let mut first = None;
    until(
        "the kubectl watch to see the pod's first pull failure",
        || arrivals.describe(),
        || {
            first = arrivals
                .first(|a| a.is_warning_about("Pod", pod) && a.reason == PULL_FAILED_REASON);
            first.is_some()
        },
    )
    .await;
    let first = first.expect("seen");
    let held = || {
        chain
            .in_namespace(ns)
            .iter()
            .any(|e| e.uid.as_deref() == Some(first.uid.as_str()))
    };
    until(
        "the service to hold the same event",
        || chain.diagnostics(ns),
        held,
    )
    .await;
    Instant::now().saturating_duration_since(first.at)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_events_k8s_io_the_service_reads_core_v1_and_normalises_the_same_events() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");

    // The reference: the real connection (events.k8s.io/v1 on this cluster).
    let reference = Chain::connect(&kind, Arc::new(kube_connector(&kind))).await;
    // The fixture: the same cluster with events.k8s.io unavailable.
    let hidden = client_without_events_group(&kind).await;
    let disabled = Chain::connect(
        &kind,
        Arc::new(EventsConnector::new(
            Arc::new(kube_connector(&kind)),
            move |cluster, _| {
                let discovery = KubeDiscovery::with_config(
                    hidden.clone(),
                    DiscoveryConfig {
                        aggregated: false,
                        ..DiscoveryConfig::default()
                    },
                );
                Arc::new(KubeEvents::new(
                    KubeResources::new(hidden.clone(), discovery),
                    cluster.clone(),
                ))
            },
        )),
    )
    .await;
    reference.ready(&ns).await;
    disabled.ready(&ns).await;
    let apis = |chain: &Chain| chain.events.status(&chain.cluster).expect("a feed").apis;
    assert_eq!(
        apis(&reference),
        [EventApi::EventsV1],
        "the cluster serves events.k8s.io"
    );
    assert_eq!(
        apis(&disabled),
        [EventApi::Core],
        "without it the feed reads core/v1"
    );

    let mut arrivals = tokio::task::block_in_place(|| {
        EventArrivals::start(kind.context.as_str(), ns.name(), DEADLINE).expect("the watch")
    });
    let created = Instant::now();
    create_in(&kind, &ns, &unpullable_pod("ev-pull"));

    // Scenario 1: live within a second of the event existing, on both APIs.
    for (api, chain) in [("events.k8s.io/v1", &reference), ("core/v1", &disabled)] {
        let late = latency(chain, &ns, &arrivals, "ev-pull").await;
        eprintln!("{api}: the Warning reached the EventService {late:?} after kubectl saw it");
        assert!(
            late <= LIVE_WITHIN,
            "{api}: the Warning reached the service {late:?} after kubectl saw it (pod created \
             {:?} ago)\n{}",
            created.elapsed(),
            chain.diagnostics(&ns)
        );
    }
    let warning = arrivals.wait_for(DEADLINE, |a| a.is_warning_about("Pod", "ev-pull"));
    assert!(warning.is_some(), "{}", arrivals.describe());

    // Scenario 4: the same normalised events either way. The kubelet keeps updating them (the
    // back-off counts), so the two feeds agree once they have both applied the latest change.
    let both = || {
        (
            about(&reference, &ns, "ev-pull"),
            about(&disabled, &ns, "ev-pull"),
        )
    };
    until(
        "the two feeds to hold identical events about the pod",
        || {
            let (v1, core) = both();
            format!("events.k8s.io/v1: {v1:#?}\ncore/v1: {core:#?}")
        },
        || {
            let (v1, core) = both();
            let warnings = v1
                .values()
                .filter(|e| e.event_type == EventType::Warning && &*e.reason == PULL_FAILED_REASON)
                .count();
            warnings >= 2 && v1 == core
        },
    )
    .await;
    let (events, _) = both();
    assert!(
        events
            .values()
            .all(|e| e.regarding_uid.is_some() && e.reporting_component.is_some()),
        "every event names its pod by uid and its reporter: {events:#?}"
    );
}
