//! The adapter's own view of a `metrics.k8s.io/v1beta1` object, whichever decoder produced it.
//!
//! Both decoders (the `k8s-metrics` types and the thin fallback over `DynamicObject`) yield
//! these plain records, so [`convert`](super::convert) and everything above it never learns
//! which one ran. Quantities stay as the server's text until [`convert`](super::convert)
//! parses them with the domain [`Quantity`](oxikube_domain::Quantity).

use jiff::{SignedDuration, Timestamp};
use k8s_metrics::v1beta1::{Container, NodeMetrics, PodMetrics, Usage};
use kube::core::DynamicObject;
use serde_json::Value;

use super::duration::parse_go_duration;

/// CPU and memory usage as the server's quantity text. `None` when the field is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RawUsage {
    pub(super) cpu: Option<String>,
    pub(super) memory: Option<String>,
}

/// One `NodeMetrics` object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawNode {
    pub(super) name: String,
    pub(super) timestamp: Option<Timestamp>,
    pub(super) window: Option<SignedDuration>,
    pub(super) usage: RawUsage,
}

/// One `PodMetrics` object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawPod {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) timestamp: Option<Timestamp>,
    pub(super) window: Option<SignedDuration>,
    pub(super) containers: Vec<RawUsage>,
}

impl From<Usage> for RawUsage {
    fn from(usage: Usage) -> Self {
        Self {
            cpu: Some(usage.cpu.0),
            memory: Some(usage.memory.0),
        }
    }
}

impl From<Container> for RawUsage {
    fn from(container: Container) -> Self {
        container.usage.into()
    }
}

fn window_of(window: std::time::Duration) -> Option<SignedDuration> {
    SignedDuration::try_from(window).ok()
}

impl From<NodeMetrics> for RawNode {
    fn from(node: NodeMetrics) -> Self {
        Self {
            name: node.metadata.name.unwrap_or_default(),
            timestamp: Some(node.timestamp.0),
            window: window_of(node.window),
            usage: node.usage.into(),
        }
    }
}

impl From<PodMetrics> for RawPod {
    fn from(pod: PodMetrics) -> Self {
        Self {
            namespace: pod.metadata.namespace.unwrap_or_default(),
            name: pod.metadata.name.unwrap_or_default(),
            timestamp: Some(pod.timestamp.0),
            window: window_of(pod.window),
            containers: pod.containers.into_iter().map(RawUsage::from).collect(),
        }
    }
}

// --- thin fallback decoder -------------------------------------------------------------------
//
// Reads the same objects out of `DynamicObject::data` with every field optional, so a server
// that sends a shape the `k8s-metrics` crate rejects (a renamed field, a duration it cannot
// parse, a container without `usage`) still yields samples, with the unreadable parts missing.

fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn usage_of(value: &Value) -> RawUsage {
    RawUsage {
        cpu: str_at(value, "cpu").map(str::to_owned),
        memory: str_at(value, "memory").map(str::to_owned),
    }
}

fn timestamp_of(data: &Value) -> Option<Timestamp> {
    str_at(data, "timestamp")?.parse().ok()
}

fn window_text(data: &Value) -> Option<SignedDuration> {
    parse_go_duration(str_at(data, "window")?)
}

impl RawNode {
    /// Decodes a `NodeMetrics` leniently from a dynamic object.
    pub(super) fn from_dynamic(object: DynamicObject) -> Self {
        let usage = object.data.get("usage").map(usage_of).unwrap_or_default();
        Self {
            name: object.metadata.name.unwrap_or_default(),
            timestamp: timestamp_of(&object.data),
            window: window_text(&object.data),
            usage,
        }
    }
}

impl RawPod {
    /// Decodes a `PodMetrics` leniently from a dynamic object.
    pub(super) fn from_dynamic(object: DynamicObject) -> Self {
        let containers = object
            .data
            .get("containers")
            .and_then(Value::as_array)
            .map(|all| {
                all.iter()
                    .map(|c| c.get("usage").map(usage_of).unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            namespace: object.metadata.namespace.unwrap_or_default(),
            name: object.metadata.name.unwrap_or_default(),
            timestamp: timestamp_of(&object.data),
            window: window_text(&object.data),
            containers,
        }
    }
}
