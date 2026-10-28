//! [`EventBuilder`]: a domain [`Event`] for tests, starting from [`event`].

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};

/// Builds a domain [`Event`]; start with [`event`].
#[derive(Debug, Clone)]
pub struct EventBuilder(Event);

/// A `Normal` event `Scheduled` about pod `default/web-0` of cluster `kubeconfig:kind-oxikube`,
/// seen once at `2026-01-01T00:00:00Z`. Change what a test cares about.
pub fn event() -> EventBuilder {
    let cluster = ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"));
    let at: Timestamp = "2026-01-01T00:00:00Z"
        .parse()
        .unwrap_or(Timestamp::UNIX_EPOCH);
    EventBuilder(Event {
        uid: Some(Arc::from("event-uid")),
        namespace: Some(Arc::from("default")),
        event_type: EventType::Normal,
        reason: Arc::from("Scheduled"),
        message: "Successfully assigned default/web-0 to node-1".into(),
        truncated: false,
        regarding: ResourceRef::new(
            cluster,
            Gvk::new("", "v1", "Pod"),
            Some("default".into()),
            "web-0",
        ),
        regarding_uid: Some(Arc::from("pod-uid")),
        related: None,
        count: 1,
        first_seen: Some(at),
        last_seen: Some(at),
        reporting_component: Some(Arc::from("default-scheduler")),
        reporting_instance: None,
        action: None,
    })
}

impl EventBuilder {
    /// The event's own uid.
    #[must_use]
    pub fn uid(mut self, uid: &str) -> Self {
        self.0.uid = Some(Arc::from(uid));
        self
    }

    /// Type `Warning`.
    #[must_use]
    pub fn warning(mut self) -> Self {
        self.0.event_type = EventType::Warning;
        self
    }

    /// The reason.
    #[must_use]
    pub fn reason(mut self, reason: &str) -> Self {
        self.0.reason = Arc::from(reason);
        self
    }

    /// The message.
    #[must_use]
    pub fn message(mut self, message: &str) -> Self {
        self.0.message = message.to_owned();
        self
    }

    /// The object the event is about (`kind` in the core group) and its uid.
    #[must_use]
    pub fn regarding(mut self, kind: &str, namespace: Option<&str>, name: &str, uid: &str) -> Self {
        let cluster = self.0.regarding.cluster.clone();
        self.0.regarding = ResourceRef::new(
            cluster,
            Gvk::new("", "v1", kind),
            namespace.map(Arc::from),
            name,
        );
        self.0.regarding_uid = Some(Arc::from(uid));
        self
    }

    /// The event's writer left the object's uid out (`involvedObject.uid` is empty): the event
    /// is only about its object by name.
    #[must_use]
    pub fn without_object_uid(mut self) -> Self {
        self.0.regarding_uid = None;
        self
    }

    /// How many times it happened.
    #[must_use]
    pub fn count(mut self, count: u32) -> Self {
        self.0.count = count;
        self
    }

    /// When it was last seen (RFC 3339).
    ///
    /// # Panics
    ///
    /// When `at` is not an RFC 3339 timestamp.
    #[must_use]
    pub fn last_seen(mut self, at: &str) -> Self {
        self.0.last_seen = Some(at.parse().expect("an RFC 3339 timestamp"));
        self
    }

    /// The event.
    pub fn build(self) -> Event {
        self.0
    }
}
