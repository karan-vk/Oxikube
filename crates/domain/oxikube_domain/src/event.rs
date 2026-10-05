//! [`Event`]: one Kubernetes event, merged from the two API shapes.
//!
//! Clusters serve events as `core/v1` `Event` (the legacy shape that kdash and
//! `kubectl get events` use) and as `events.k8s.io/v1` `Event` (the newer
//! shape that controllers write with `eventTime` and `series`). Both describe
//! the same thing, so the UI, the events feed and agent context work on this
//! one struct. The mapping functions are JSON driven ([`Event::from_core_v1`],
//! [`Event::from_events_v1`], [`Event::from_json`]) so the domain keeps no
//! dependency on `k8s-openapi`.
//!
//! # Field mapping
//!
//! | `Event` field | `core/v1` | `events.k8s.io/v1` |
//! |---|---|---|
//! | `uid` | `metadata.uid` | `metadata.uid` |
//! | `event_type` | `type` | `type` |
//! | `reason` | `reason` | `reason` |
//! | `message` | `message` | `note` |
//! | `regarding` | `involvedObject` | `regarding` |
//! | `regarding_uid` | `involvedObject.uid` | `regarding.uid` |
//! | `related` | `related` | `related` |
//! | `count` | `series.count`, else `count`, else 1 | `series.count`, else `deprecatedCount`, else 1 |
//! | `first_seen` | `firstTimestamp`, else `eventTime`, else `metadata.creationTimestamp` | `deprecatedFirstTimestamp`, else `eventTime`, else `metadata.creationTimestamp` |
//! | `last_seen` | `series.lastObservedTime`, else `lastTimestamp`, else `eventTime`, else `first_seen` | `series.lastObservedTime`, else `deprecatedLastTimestamp`, else `eventTime`, else `first_seen` |
//! | `reporting_component` | `reportingComponent`, else `source.component` | `reportingController`, else `deprecatedSource.component` |
//! | `reporting_instance` | `reportingInstance`, else `source.host` | `reportingInstance`, else `deprecatedSource.host` |
//! | `action` | `action` | `action` |
//!
//! Empty strings count as absent. The event's own namespace and name are not
//! kept: events are identified by `uid` and shown against `regarding`.
//!
//! # Bounds
//!
//! `message` is capped at [`MAX_EVENT_MESSAGE_BYTES`] on a char boundary and
//! [`Event::truncated`] records the cut. Like every record here, an `Event`
//! is not redacted by the domain.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bounds::truncate_in_place;
use crate::ids::{ClusterId, Gvk, ResourceRef};

/// Longest event message kept, in bytes (4 KiB). The API caps `note` at 1 KiB,
/// but older and third-party writers are not so careful.
pub const MAX_EVENT_MESSAGE_BYTES: usize = 4 * 1024;

/// Severity class of an [`Event`] (`type` in the API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    /// Routine information.
    Normal,
    /// Something went wrong or may go wrong.
    Warning,
    /// The writer used a `type` other than `Normal` or `Warning`.
    Other,
}

impl EventType {
    /// Map the API `type` string; unknown or absent values become [`EventType::Other`].
    pub fn from_api(s: &str) -> Self {
        match s {
            "Normal" => Self::Normal,
            "Warning" => Self::Warning,
            _ => Self::Other,
        }
    }
}

/// One Kubernetes event in the merged `core/v1` + `events.k8s.io/v1` shape.
///
/// See the [module docs](self) for the field mapping. Field names are stable:
/// the type is persisted and shown to agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// `metadata.uid`; distinguishes events that share a reason and object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<Arc<str>>,
    /// Severity class. Serialised as `type`.
    #[serde(rename = "type")]
    pub event_type: EventType,
    /// Short machine-readable reason, for example `BackOff`.
    pub reason: Arc<str>,
    /// Human-readable description, at most [`MAX_EVENT_MESSAGE_BYTES`] long.
    pub message: String,
    /// Whether `message` was cut to fit [`MAX_EVENT_MESSAGE_BYTES`].
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    /// The object the event is about.
    pub regarding: ResourceRef,
    /// `uid` of the object the event is about, when the writer set it. Lets a feed
    /// filter and an index group by object even after the object is renamed or re-created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regarding_uid: Option<Arc<str>>,
    /// A secondary object, when the writer gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related: Option<ResourceRef>,
    /// How many times this event occurred; at least 1.
    pub count: u32,
    /// When the event was first seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<Timestamp>,
    /// When the event was last seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<Timestamp>,
    /// The controller or component that reported it, for example `kubelet`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporting_component: Option<Arc<str>>,
    /// The instance of the reporter, usually a node or pod name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporting_instance: Option<Arc<str>>,
    /// What was being done to the object, when the writer said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Arc<str>>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Why a JSON document could not be mapped to an [`Event`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventParseError {
    /// The document is not a JSON object.
    #[error("event is not a JSON object")]
    NotAnObject,
    /// A required field is missing or empty.
    #[error("event is missing required field `{0}`")]
    MissingField(&'static str),
    /// A field that must be a timestamp is not RFC 3339.
    #[error("event field `{field}` is not an RFC 3339 timestamp: {value:?}")]
    BadTimestamp {
        /// JSON field name.
        field: &'static str,
        /// The offending value.
        value: String,
    },
}

/// Which API shape a document is in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Core,
    EventsV1,
}

/// Field names that differ between the two shapes.
struct Names {
    message: &'static str,
    regarding: &'static str,
    count: &'static str,
    first: &'static str,
    last: &'static str,
    component: &'static str,
    legacy_source: &'static str,
}

impl Shape {
    fn names(self) -> Names {
        match self {
            Shape::Core => Names {
                message: "message",
                regarding: "involvedObject",
                count: "count",
                first: "firstTimestamp",
                last: "lastTimestamp",
                component: "reportingComponent",
                legacy_source: "source",
            },
            Shape::EventsV1 => Names {
                message: "note",
                regarding: "regarding",
                count: "deprecatedCount",
                first: "deprecatedFirstTimestamp",
                last: "deprecatedLastTimestamp",
                component: "reportingController",
                legacy_source: "deprecatedSource",
            },
        }
    }
}

impl Event {
    /// Map a `core/v1` `Event` JSON object. `cluster` is the cluster it was read from.
    pub fn from_core_v1(cluster: &ClusterId, json: &Value) -> Result<Self, EventParseError> {
        Self::map(cluster, json, Shape::Core)
    }

    /// Map an `events.k8s.io/v1` `Event` JSON object.
    pub fn from_events_v1(cluster: &ClusterId, json: &Value) -> Result<Self, EventParseError> {
        Self::map(cluster, json, Shape::EventsV1)
    }

    /// Map either shape, picking by `apiVersion` and falling back to the
    /// presence of `involvedObject` (core) or `regarding` (events.k8s.io).
    /// Documents that match neither are read as `core/v1`.
    pub fn from_json(cluster: &ClusterId, json: &Value) -> Result<Self, EventParseError> {
        let shape = match json.get("apiVersion").and_then(Value::as_str) {
            Some("events.k8s.io/v1") | Some("events.k8s.io/v1beta1") => Shape::EventsV1,
            Some("v1") => Shape::Core,
            _ if json.get("involvedObject").is_some() => Shape::Core,
            _ if json.get("regarding").is_some() => Shape::EventsV1,
            _ => Shape::Core,
        };
        Self::map(cluster, json, shape)
    }

    fn map(cluster: &ClusterId, json: &Value, shape: Shape) -> Result<Self, EventParseError> {
        let obj = json.as_object().ok_or(EventParseError::NotAnObject)?;
        let n = shape.names();

        let regarding = object_ref(cluster, obj.get(n.regarding))
            .ok_or(EventParseError::MissingField(n.regarding))?;
        let regarding_uid = obj
            .get(n.regarding)
            .and_then(|r| str_field(r, "uid"))
            .map(Arc::from);
        let related = object_ref(cluster, obj.get("related"));

        let mut message = str_field(json, n.message).unwrap_or_default().to_owned();
        let truncated = truncate_in_place(&mut message, MAX_EVENT_MESSAGE_BYTES);

        let series = json.get("series");
        let count = series
            .and_then(|s| s.get("count"))
            .or_else(|| json.get(n.count))
            .and_then(Value::as_u64)
            .map(|c| u32::try_from(c).unwrap_or(u32::MAX).max(1))
            .unwrap_or(1);

        let event_time = ts_field(json, "eventTime")?;
        let created = match json.get("metadata") {
            Some(m) => ts_field(m, "creationTimestamp")?,
            None => None,
        };
        let first_seen = ts_field(json, n.first)?.or(event_time).or(created);
        let last_seen = match series {
            Some(s) => ts_field(s, "lastObservedTime")?,
            None => None,
        }
        .or(ts_field(json, n.last)?)
        .or(event_time)
        .or(first_seen);

        let source = json.get(n.legacy_source);
        let reporting_component = str_field(json, n.component)
            .or_else(|| source.and_then(|s| str_field(s, "component")))
            .map(Arc::from);
        let reporting_instance = str_field(json, "reportingInstance")
            .or_else(|| source.and_then(|s| str_field(s, "host")))
            .map(Arc::from);

        Ok(Self {
            uid: json
                .get("metadata")
                .and_then(|m| str_field(m, "uid"))
                .map(Arc::from),
            event_type: EventType::from_api(str_field(json, "type").unwrap_or_default()),
            reason: Arc::from(str_field(json, "reason").unwrap_or_default()),
            message,
            truncated,
            regarding,
            regarding_uid,
            related,
            count,
            first_seen,
            last_seen,
            reporting_component,
            reporting_instance,
            action: str_field(json, "action").map(Arc::from),
        })
    }

    /// Whether this is a [`EventType::Warning`].
    pub fn is_warning(&self) -> bool {
        self.event_type == EventType::Warning
    }
}

/// A non-empty string field, if present.
fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// An RFC 3339 timestamp field; absent and empty are `None`, malformed is an error.
fn ts_field(v: &Value, key: &'static str) -> Result<Option<Timestamp>, EventParseError> {
    match str_field(v, key) {
        None => Ok(None),
        Some(s) => s
            .parse::<Timestamp>()
            .map(Some)
            .map_err(|_| EventParseError::BadTimestamp {
                field: key,
                value: s.to_owned(),
            }),
    }
}

/// Build a [`ResourceRef`] from an `ObjectReference`; needs at least `kind` and `name`.
fn object_ref(cluster: &ClusterId, v: Option<&Value>) -> Option<ResourceRef> {
    let v = v?;
    let kind = str_field(v, "kind")?;
    let name = str_field(v, "name")?;
    let gvk = Gvk::from_api_version(str_field(v, "apiVersion").unwrap_or("v1"), kind);
    Some(ResourceRef::new(
        cluster.clone(),
        gvk,
        str_field(v, "namespace").map(Arc::from),
        name,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ContextName;
    use proptest::prelude::*;
    use serde_json::json;

    fn cluster() -> ClusterId {
        ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"))
    }

    fn core_fixture() -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "Event",
            "metadata": {
                "name": "web-0.17a",
                "namespace": "default",
                "uid": "11111111-2222-3333-4444-555555555555",
                "creationTimestamp": "2026-10-03T11:00:00Z"
            },
            "involvedObject": {
                "kind": "Pod",
                "namespace": "default",
                "name": "web-0",
                "apiVersion": "v1",
                "uid": "abc"
            },
            "reason": "BackOff",
            "message": "Back-off restarting failed container",
            "source": {"component": "kubelet", "host": "worker-1"},
            "firstTimestamp": "2026-10-03T11:00:00Z",
            "lastTimestamp": "2026-10-03T11:05:00Z",
            "count": 7,
            "type": "Warning"
        })
    }

    fn events_v1_fixture() -> Value {
        json!({
            "apiVersion": "events.k8s.io/v1",
            "kind": "Event",
            "metadata": {
                "name": "web-0.17a",
                "namespace": "default",
                "uid": "11111111-2222-3333-4444-555555555555",
                "creationTimestamp": "2026-10-03T11:00:00Z"
            },
            "eventTime": "2026-10-03T11:00:00.123456Z",
            "regarding": {
                "kind": "Pod",
                "namespace": "default",
                "name": "web-0",
                "apiVersion": "v1",
                "uid": "abc"
            },
            "reason": "BackOff",
            "note": "Back-off restarting failed container",
            "reportingController": "kubelet",
            "reportingInstance": "worker-1",
            "action": "Restarting",
            "series": {"count": 7, "lastObservedTime": "2026-10-03T11:05:00.000000Z"},
            "type": "Warning"
        })
    }

    #[test]
    fn core_v1_maps_every_field() {
        let e = Event::from_core_v1(&cluster(), &core_fixture()).unwrap();
        assert_eq!(
            e.uid.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(e.event_type, EventType::Warning);
        assert!(e.is_warning());
        assert_eq!(&*e.reason, "BackOff");
        assert_eq!(e.message, "Back-off restarting failed container");
        assert!(!e.truncated);
        assert_eq!(e.regarding.gvk, Gvk::from_api_version("v1", "Pod"));
        assert_eq!(e.regarding.namespace(), Some("default"));
        assert_eq!(&*e.regarding.name, "web-0");
        assert_eq!(e.regarding.cluster, cluster());
        assert_eq!(e.regarding_uid.as_deref(), Some("abc"));
        assert!(e.related.is_none());
        assert_eq!(e.count, 7);
        assert_eq!(e.first_seen, Some("2026-10-03T11:00:00Z".parse().unwrap()));
        assert_eq!(e.last_seen, Some("2026-10-03T11:05:00Z".parse().unwrap()));
        assert_eq!(e.reporting_component.as_deref(), Some("kubelet"));
        assert_eq!(e.reporting_instance.as_deref(), Some("worker-1"));
        assert_eq!(e.action, None);
    }

    #[test]
    fn events_v1_maps_every_field() {
        let e = Event::from_events_v1(&cluster(), &events_v1_fixture()).unwrap();
        assert_eq!(e.event_type, EventType::Warning);
        assert_eq!(&*e.reason, "BackOff");
        assert_eq!(e.message, "Back-off restarting failed container");
        assert_eq!(&*e.regarding.name, "web-0");
        assert_eq!(e.regarding_uid.as_deref(), Some("abc"));
        assert_eq!(e.count, 7);
        assert_eq!(
            e.first_seen,
            Some("2026-10-03T11:00:00.123456Z".parse().unwrap())
        );
        assert_eq!(e.last_seen, Some("2026-10-03T11:05:00Z".parse().unwrap()));
        assert_eq!(e.reporting_component.as_deref(), Some("kubelet"));
        assert_eq!(e.reporting_instance.as_deref(), Some("worker-1"));
        assert_eq!(e.action.as_deref(), Some("Restarting"));
    }

    #[test]
    fn both_shapes_feed_the_same_struct() {
        let core = Event::from_core_v1(&cluster(), &core_fixture()).unwrap();
        let v1 = Event::from_events_v1(&cluster(), &events_v1_fixture()).unwrap();
        // Same logical event: identity, severity, subject, count and reporter agree.
        assert_eq!(core.uid, v1.uid);
        assert_eq!(core.event_type, v1.event_type);
        assert_eq!(core.reason, v1.reason);
        assert_eq!(core.message, v1.message);
        assert_eq!(core.regarding, v1.regarding);
        assert_eq!(core.count, v1.count);
        assert_eq!(core.last_seen, v1.last_seen);
        assert_eq!(core.reporting_component, v1.reporting_component);
    }

    #[test]
    fn from_json_detects_the_shape() {
        let c = cluster();
        let core = Event::from_json(&c, &core_fixture()).unwrap();
        assert_eq!(core, Event::from_core_v1(&c, &core_fixture()).unwrap());
        let v1 = Event::from_json(&c, &events_v1_fixture()).unwrap();
        assert_eq!(v1, Event::from_events_v1(&c, &events_v1_fixture()).unwrap());

        // No apiVersion: decided by the object-reference field.
        let mut no_version = events_v1_fixture();
        no_version.as_object_mut().unwrap().remove("apiVersion");
        assert_eq!(Event::from_json(&c, &no_version).unwrap(), v1);
        let mut no_version = core_fixture();
        no_version.as_object_mut().unwrap().remove("apiVersion");
        assert_eq!(Event::from_json(&c, &no_version).unwrap(), core);
    }

    #[test]
    fn legacy_core_fields_fall_back() {
        let doc = json!({
            "involvedObject": {"kind": "Node", "name": "worker-1"},
            "reason": "NodeReady",
            "message": "ok",
            "type": "Normal",
            "eventTime": "2026-10-03T11:00:00Z",
            "reportingComponent": "node-controller",
            "reportingInstance": "cp-0",
        });
        let e = Event::from_core_v1(&cluster(), &doc).unwrap();
        assert_eq!(e.count, 1);
        assert_eq!(e.first_seen, e.last_seen);
        assert_eq!(e.reporting_component.as_deref(), Some("node-controller"));
        assert_eq!(e.reporting_instance.as_deref(), Some("cp-0"));
        assert_eq!(e.regarding.namespace(), None, "node is cluster scoped");
        assert_eq!(e.event_type, EventType::Normal);
    }

    #[test]
    fn related_object_is_mapped() {
        let mut doc = events_v1_fixture();
        doc["related"] = json!({"kind": "Node", "name": "worker-1"});
        let e = Event::from_events_v1(&cluster(), &doc).unwrap();
        let related = e.related.unwrap();
        assert_eq!(related.gvk.kind.as_ref(), "Node");
        assert_eq!(&*related.name, "worker-1");
    }

    #[test]
    fn unknown_type_is_other() {
        let mut doc = core_fixture();
        doc["type"] = json!("Surprise");
        let e = Event::from_core_v1(&cluster(), &doc).unwrap();
        assert_eq!(e.event_type, EventType::Other);
        doc.as_object_mut().unwrap().remove("type");
        let e = Event::from_core_v1(&cluster(), &doc).unwrap();
        assert_eq!(e.event_type, EventType::Other);
    }

    #[test]
    fn errors_are_typed() {
        let c = cluster();
        assert_eq!(
            Event::from_json(&c, &json!([])),
            Err(EventParseError::NotAnObject)
        );
        assert_eq!(
            Event::from_core_v1(&c, &json!({"reason": "x"})),
            Err(EventParseError::MissingField("involvedObject"))
        );
        assert_eq!(
            Event::from_events_v1(&c, &json!({"regarding": {"kind": "Pod"}})),
            Err(EventParseError::MissingField("regarding"))
        );
        let mut doc = core_fixture();
        doc["firstTimestamp"] = json!("yesterday");
        assert!(matches!(
            Event::from_core_v1(&c, &doc),
            Err(EventParseError::BadTimestamp {
                field: "firstTimestamp",
                ..
            })
        ));
    }

    #[test]
    fn long_message_is_cut_and_flagged() {
        let mut doc = core_fixture();
        doc["message"] = json!("é".repeat(MAX_EVENT_MESSAGE_BYTES));
        let e = Event::from_core_v1(&cluster(), &doc).unwrap();
        assert!(e.truncated);
        assert!(e.message.len() <= MAX_EVENT_MESSAGE_BYTES);
    }

    #[test]
    fn huge_count_saturates() {
        let mut doc = core_fixture();
        doc["count"] = json!(u64::MAX);
        assert_eq!(
            Event::from_core_v1(&cluster(), &doc).unwrap().count,
            u32::MAX
        );
        doc["count"] = json!(0);
        assert_eq!(Event::from_core_v1(&cluster(), &doc).unwrap().count, 1);
    }

    #[test]
    fn serde_round_trip_full_and_minimal() {
        let full = Event::from_events_v1(&cluster(), &events_v1_fixture()).unwrap();
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["type"], "Warning");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), full);

        let minimal = Event::from_core_v1(
            &cluster(),
            &json!({"involvedObject": {"kind": "Pod", "name": "p"}}),
        )
        .unwrap();
        assert!(minimal.uid.is_none() && minimal.first_seen.is_none());
        assert!(minimal.regarding_uid.is_none());
        let json = serde_json::to_value(&minimal).unwrap();
        for absent in [
            "uid",
            "regarding_uid",
            "related",
            "first_seen",
            "last_seen",
            "action",
            "truncated",
        ] {
            assert!(json.get(absent).is_none(), "{absent} should be omitted");
        }
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), minimal);
    }

    proptest! {
        #[test]
        fn arbitrary_message_never_panics(msg in any::<String>()) {
            let mut doc = core_fixture();
            doc["message"] = json!(msg);
            let e = Event::from_core_v1(&cluster(), &doc).unwrap();
            prop_assert!(e.message.len() <= MAX_EVENT_MESSAGE_BYTES);
            prop_assert!(msg.starts_with(&e.message));
        }
    }
}
