//! Decorators that narrow the events feeds of a real connection, for the kind suites (E14-S10).
//!
//! The app's `EventService` always opens one cluster-wide feed. On the shared kind cluster that
//! feed carries every suite's events, so a test asserting exact counts (one notification per
//! pod, one group row for a burst) would see its neighbours' pods too. [`ScopedEvents`] wraps a
//! cluster's [`EventsPort`] and rewrites what each feed watches: the test's own namespaces in
//! place of the cluster, and, optionally, a forced [`EventApiChoice`] (`CoreOnly` is the hook
//! that forces the `core/v1` fallback). [`EventsConnector`] applies such a wrapper (or any other)
//! to every connection a real connector makes, so a session manager built on it hands the
//! service the narrowed port. Everything else on the connection is untouched.
//!
//! ```ignore
//! let connector = EventsConnector::new(kube_connector, move |_, events| {
//!     Arc::new(ScopedEvents::new(events).namespaces([ns.name()]))
//! });
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{
    ClusterConnection, ClusterConnectorPort, ConnectRequest, EventApiChoice, EventsFeed,
    EventsPort, EventsWatchOptions,
};

/// An [`EventsPort`] that rewrites the scope and API of every feed it opens. See the
/// [module docs](self).
pub struct ScopedEvents {
    inner: Arc<dyn EventsPort>,
    namespaces: Option<Vec<String>>,
    api: Option<EventApiChoice>,
}

impl ScopedEvents {
    /// Passes every feed through to `inner` unchanged until narrowed.
    pub fn new(inner: Arc<dyn EventsPort>) -> Self {
        Self {
            inner,
            namespaces: None,
            api: None,
        }
    }

    /// Watches only `namespaces`, whatever scope the caller asked for.
    #[must_use]
    pub fn namespaces<S: Into<String>>(mut self, namespaces: impl IntoIterator<Item = S>) -> Self {
        self.namespaces = Some(namespaces.into_iter().map(Into::into).collect());
        self
    }

    /// Watches `api`, whatever the caller chose (`CoreOnly`: the `core/v1` fallback).
    #[must_use]
    pub fn api(mut self, api: EventApiChoice) -> Self {
        self.api = Some(api);
        self
    }

    /// The scope and options a feed asked with `scope` and `options` is opened with.
    fn rewrite(
        &self,
        scope: &WatchScope,
        options: &EventsWatchOptions,
    ) -> (WatchScope, EventsWatchOptions) {
        let scope = match &self.namespaces {
            Some(names) => WatchScope::Namespaces(names.clone()),
            None => scope.clone(),
        };
        let mut options = options.clone();
        if let Some(api) = self.api {
            options.api = api;
        }
        if self.namespaces.is_some() {
            // A namespace scope never falls back (the adapter refuses fallbacks for it).
            options.fallback_namespaces.clear();
        }
        (scope, options)
    }
}

#[async_trait]
impl EventsPort for ScopedEvents {
    async fn watch_events(
        &self,
        scope: &WatchScope,
        options: &EventsWatchOptions,
    ) -> OxiResult<EventsFeed> {
        let (scope, options) = self.rewrite(scope, options);
        self.inner.watch_events(&scope, &options).await
    }
}

/// How [`EventsConnector`] replaces a connection's events port: given the cluster and the
/// adapter's own port, the port the session gets.
pub type WrapEvents = dyn Fn(&ClusterId, Arc<dyn EventsPort>) -> Arc<dyn EventsPort> + Send + Sync;

/// A [`ClusterConnectorPort`] that connects through `inner` and replaces each connection's
/// events port with what `wrap` makes of it. See the [module docs](self).
pub struct EventsConnector {
    inner: Arc<dyn ClusterConnectorPort>,
    wrap: Box<WrapEvents>,
}

impl EventsConnector {
    /// Connects through `inner`, wrapping the events port of every connection with `wrap`.
    pub fn new(
        inner: Arc<dyn ClusterConnectorPort>,
        wrap: impl Fn(&ClusterId, Arc<dyn EventsPort>) -> Arc<dyn EventsPort> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner,
            wrap: Box::new(wrap),
        }
    }
}

#[async_trait]
impl ClusterConnectorPort for EventsConnector {
    async fn connect(&self, request: ConnectRequest) -> OxiResult<ClusterConnection> {
        let cluster = request.cluster.clone();
        let mut connection = self.inner.connect(request).await?;
        let events = connection.ports.events.clone();
        connection.ports.events = (self.wrap)(&cluster, events);
        Ok(connection)
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::ContextName;
    use oxikube_ports::{ExecInteractivity, HealthReporter, HealthSignal};

    use super::*;
    use crate::{EventsCall, FakeClusterConnectorPort, FakeEventFeed, FakeEventsPort};

    struct Ignore;

    impl HealthReporter for Ignore {
        fn report(&self, _: HealthSignal) {}
    }

    #[test]
    fn the_scope_and_the_api_are_rewritten_and_the_rest_kept() {
        let port = ScopedEvents::new(Arc::new(FakeEventsPort::new()))
            .namespaces(["oxi-test-a"])
            .api(EventApiChoice::CoreOnly);
        let asked = EventsWatchOptions::default()
            .capacity(7)
            .fallback_namespaces(["x".to_owned()]);
        let (scope, options) = port.rewrite(&WatchScope::Cluster, &asked);
        assert_eq!(scope, WatchScope::Namespaces(vec!["oxi-test-a".into()]));
        assert_eq!(options.api, EventApiChoice::CoreOnly);
        assert_eq!(options.capacity, Some(7));
        assert!(options.fallback_namespaces.is_empty());
    }

    #[test]
    fn an_unnarrowed_port_passes_everything_through() {
        let port = ScopedEvents::new(Arc::new(FakeEventsPort::new()));
        let asked = EventsWatchOptions::default().fallback_namespaces(["x".to_owned()]);
        let (scope, options) = port.rewrite(&WatchScope::Cluster, &asked);
        assert_eq!(scope, WatchScope::Cluster);
        assert_eq!(options, asked);
    }

    #[test]
    fn the_connector_hands_the_session_the_wrapped_port() {
        let inner = Arc::new(FakeClusterConnectorPort::new());
        let cluster = ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"));
        let fake = inner.ports_for(&cluster).events;
        fake.install(FakeEventFeed::new());
        let connector = EventsConnector::new(inner, |_, events| {
            Arc::new(ScopedEvents::new(events).namespaces(["oxi-test-b"]))
        });
        let request = ConnectRequest {
            cluster,
            context: ContextName::new("kind-oxikube"),
            exec_interactivity: ExecInteractivity::Never,
            health: Arc::new(Ignore),
        };
        let connection =
            futures::executor::block_on(connector.connect(request)).expect("connected");
        futures::executor::block_on(
            connection
                .ports
                .events
                .watch_events(&WatchScope::Cluster, &EventsWatchOptions::default()),
        )
        .expect("a feed");
        let calls = fake.recorded_calls();
        assert!(
            matches!(
                calls.as_slice(),
                [EventsCall::Watch(WatchScope::Namespaces(names), _)] if names == &["oxi-test-b"]
            ),
            "the fake saw the narrowed scope: {calls:?}"
        );
    }
}
