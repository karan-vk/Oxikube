//! Tests of the events tool, `@events` and "Send to agent" over fake sessions, a scripted events
//! feed and the real `EventService`. Tokio test runtime; every wait is on a condition with a
//! deadline, never a sleep before an assertion.

mod inventory;
mod provider;
mod send;
mod tool;

use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::Resource;
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId, ToolContext, ToolOutput};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeEventsPort,
    event,
};
use serde_json::{Value, json};

use super::{EventsAgentDeps, install, register_commands};
use crate::command_bus::{CommandBus, CommandRegistry};
use crate::context::{ContextRegistry, PendingContext};
use crate::events::{EventService, EventsConfig, EventsState};
use crate::guard::MutationGuard;
use crate::search::aliases::AliasRegistry;
use crate::session::ClusterSessionManager;
use crate::store::Spawner;
use crate::tools::ToolRegistry;

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("dev"))
}

pub(super) fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().expect("a timestamp")
}

/// When the tools run: ten minutes after [`t0`].
pub(super) fn now() -> Timestamp {
    t0() + SignedDuration::from_secs(600)
}

/// An event `uid` about `kind` `ns/name` (object uid `object_uid`), last seen `secs` after
/// [`t0`], `Normal` with reason `Pulled`.
pub(super) fn ev(
    uid: &str,
    kind: &str,
    ns: &str,
    name: &str,
    object_uid: &str,
    secs: i64,
) -> Event {
    let mut e = event()
        .uid(uid)
        .reason("Pulled")
        .message(&format!("message of {uid}"))
        .regarding(kind, Some(ns), name, object_uid)
        .build();
    e.namespace = Some(Arc::from(ns));
    e.first_seen = Some(t0() + SignedDuration::from_secs(secs));
    e.last_seen = Some(t0() + SignedDuration::from_secs(secs));
    e
}

/// [`ev`] as a `Warning` with `reason`.
pub(super) fn warning(
    uid: &str,
    kind: &str,
    ns: &str,
    name: &str,
    object_uid: &str,
    secs: i64,
    reason: &str,
) -> Event {
    let mut e = ev(uid, kind, ns, name, object_uid, secs);
    e.event_type = EventType::Warning;
    e.reason = Arc::from(reason);
    e
}

/// A pod with a uid, as the cluster would return it.
pub(super) fn pod_object(ns: &str, name: &str, uid: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": ns, "uid": uid},
    }))
    .expect("a pod")
}

fn tokio_spawner() -> Arc<dyn Spawner> {
    Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
        tokio::spawn(task);
    })
}

/// A connected fake cluster `dev` with the real `EventService` over a scripted feed.
pub(super) struct Harness {
    pub(super) sessions: ClusterSessionManager,
    pub(super) connector: Arc<FakeClusterConnectorPort>,
    pub(super) events: EventService,
    pub(super) pending: PendingContext,
    pub(super) tools: ToolRegistry,
    pub(super) contexts: ContextRegistry,
    pub(super) deps: EventsAgentDeps,
}

impl Harness {
    /// A cluster `dev`, not connected yet.
    pub(super) fn new() -> Self {
        let entry = ClusterContext::new(
            cluster(),
            ContextName::new("dev"),
            SourceId("kubeconfig".into()),
        );
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let events = EventService::new(
            sessions.clone(),
            Arc::new(FakeClockPort::new(t0())),
            EventsConfig {
                buffer: 500,
                ..EventsConfig::default()
            },
        );
        events.start(tokio_spawner());
        let pending = PendingContext::new();
        let deps = EventsAgentDeps {
            sessions: sessions.clone(),
            events: events.clone(),
            aliases: AliasRegistry::new(),
            clock: Arc::new(FakeClockPort::new(now())),
        };
        let tools = ToolRegistry::new();
        let contexts = ContextRegistry::new();
        install(&tools, &contexts, &deps).expect("install");
        Self {
            sessions,
            connector,
            events,
            pending,
            tools,
            contexts,
            deps,
        }
    }

    pub(super) async fn connect(&self) {
        self.sessions.connect(&cluster()).await.expect("connect");
    }

    /// The scripted events port of `dev`.
    pub(super) fn port(&self) -> Arc<FakeEventsPort> {
        self.connector.ports_for(&cluster()).events
    }

    /// The objects `dev` serves.
    pub(super) fn objects(&self) -> Arc<oxikube_testkit::FakeResourcePort> {
        self.connector.ports_for(&cluster()).resources
    }

    /// Connects, with `events` as the cluster's events.
    pub(super) async fn with_events(events: impl IntoIterator<Item = Event>) -> Self {
        let h = Self::new();
        h.connect().await;
        h.port().install(FakeEventFeed::new().initial(events));
        h
    }

    /// Calls `k8s.get_events` for the only connected cluster.
    pub(super) async fn get_events(&self, args: Value) -> ToolOutput {
        self.tools
            .invoke("k8s.get_events", args, &ToolContext::agent())
            .await
            .expect("the call is well-formed")
    }

    /// A command bus with `events::SendToAgent` registered.
    pub(super) fn bus(&self) -> CommandBus {
        let mut registry = CommandRegistry::new();
        registry
            .install("oxikube_app::tools::events", |r| {
                register_commands(r, &self.deps, self.pending.clone())
            })
            .expect("the command registers");
        let guard = MutationGuard::new(
            self.sessions.clone(),
            Arc::new(oxikube_testkit::FakeStatePort::new()),
            Arc::new(FakeClockPort::default()),
        );
        CommandBus::new(registry, guard)
    }

    pub(super) fn state(&self) -> EventsState {
        self.events.state(&cluster())
    }
}

/// The structured result of `out`.
pub(super) fn structured(out: &ToolOutput) -> &Value {
    out.structured.as_ref().expect("a structured result")
}

/// The text of `out`.
pub(super) fn text(out: &ToolOutput) -> String {
    out.content
        .iter()
        .filter_map(|part| match part {
            oxikube_ports::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The reasons of the rows of a `k8s.get_events` result, in order.
pub(super) fn reasons(out: &ToolOutput) -> Vec<String> {
    structured(out)["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["reason"].as_str().expect("reason").to_owned())
        .collect()
}
