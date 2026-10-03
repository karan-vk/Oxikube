//! [`MetricsSample`]: one point-in-time CPU and memory reading.
//!
//! Samples come from metrics-server (`metrics.k8s.io`, ADR 0011) and feed the
//! live table columns. The adapter owns the wire types; this record is what
//! crosses the port, so it does not use `k8s-metrics` types.
//!
//! # Units
//!
//! CPU is in **nanocores** and memory in **bytes**, both plain `u64`. These are
//! the units metrics-server reports (`n` and `Ki` suffixes parsed exactly), so
//! no precision is lost. They can move to `Quantity` without changing the wire
//! form if that proves useful.
//!
//! # Missing metrics
//!
//! ADR 0011: absence is a visible state, never a swallowed error. Each
//! reading is a [`Reading`], which is either a value or a [`MissingReason`]
//! the UI renders ("metrics-server not installed", "no permission", ...). A
//! sample with both readings missing is still a valid, useful record.

use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::ids::ResourceRef;

/// Why a metric has no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingReason {
    /// The cluster does not serve `metrics.k8s.io` (metrics-server is not installed).
    NotInstalled,
    /// The API is served but has no data for this subject yet (new pod, scrape pending).
    NotYetAvailable,
    /// The user may not read metrics for this subject.
    Forbidden,
    /// The metrics request failed for another reason.
    Unavailable,
}

/// A metric value, or the visible reason there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reading {
    /// The measured value, in the unit of the field that holds it.
    Value(u64),
    /// No value, and why.
    Missing(MissingReason),
}

impl Reading {
    /// The value, if there is one.
    pub fn value(self) -> Option<u64> {
        match self {
            Reading::Value(v) => Some(v),
            Reading::Missing(_) => None,
        }
    }

    /// The reason there is no value, if there is none.
    pub fn missing(self) -> Option<MissingReason> {
        match self {
            Reading::Value(_) => None,
            Reading::Missing(r) => Some(r),
        }
    }
}

/// What a [`MetricsSample`] measures.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetricsSubject {
    /// A node; `node` is its name.
    Node {
        /// Node name.
        node: Arc<str>,
    },
    /// A whole pod, summed over its containers.
    Pod {
        /// The pod.
        pod: ResourceRef,
    },
    /// One container of a pod.
    Container {
        /// The pod the container belongs to.
        pod: ResourceRef,
        /// Container name.
        container: Arc<str>,
    },
}

/// One CPU and memory reading for a node, pod or container.
///
/// Field names are stable: the type is persisted and shown to agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsSample {
    /// What was measured.
    pub subject: MetricsSubject,
    /// When the reading was taken (`timestamp` in the metrics API).
    pub ts: Timestamp,
    /// Length of the window the reading averages over (`window` in the metrics API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SignedDuration>,
    /// CPU usage in nanocores (1 core = 1 000 000 000).
    pub cpu: Reading,
    /// Working-set memory in bytes.
    pub memory: Reading,
}

const NANOCORES_PER_MILLICORE: u64 = 1_000_000;

impl MetricsSample {
    /// A sample with both readings present.
    pub fn new(
        subject: MetricsSubject,
        ts: Timestamp,
        window: Option<SignedDuration>,
        cpu_nanocores: u64,
        memory_bytes: u64,
    ) -> Self {
        Self {
            subject,
            ts,
            window,
            cpu: Reading::Value(cpu_nanocores),
            memory: Reading::Value(memory_bytes),
        }
    }

    /// A sample with neither reading, carrying the reason (ADR 0011).
    pub fn missing(subject: MetricsSubject, ts: Timestamp, reason: MissingReason) -> Self {
        Self {
            subject,
            ts,
            window: None,
            cpu: Reading::Missing(reason),
            memory: Reading::Missing(reason),
        }
    }

    /// CPU usage in whole millicores, rounded down, if present.
    pub fn cpu_millicores(&self) -> Option<u64> {
        self.cpu.value().map(|n| n / NANOCORES_PER_MILLICORE)
    }

    /// Memory usage in bytes, if present.
    pub fn memory_bytes(&self) -> Option<u64> {
        self.memory.value()
    }

    /// Whether either reading is missing.
    pub fn is_partial(&self) -> bool {
        self.cpu.value().is_none() || self.memory.value().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ClusterId, ContextName, Gvk};

    fn pod() -> ResourceRef {
        let cluster = ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"));
        ResourceRef::namespaced(
            cluster,
            Gvk::from_api_version("v1", "Pod"),
            "default",
            "web-0",
        )
    }

    fn ts() -> Timestamp {
        "2026-10-03T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn accessors() {
        let s = MetricsSample::new(
            MetricsSubject::Pod { pod: pod() },
            ts(),
            Some(SignedDuration::from_secs(30)),
            1_234_567_890,
            64 * 1024 * 1024,
        );
        assert_eq!(s.cpu_millicores(), Some(1234));
        assert_eq!(s.memory_bytes(), Some(67_108_864));
        assert!(!s.is_partial());
    }

    #[test]
    fn missing_is_a_visible_state() {
        let s = MetricsSample::missing(
            MetricsSubject::Node {
                node: "worker-1".into(),
            },
            ts(),
            MissingReason::NotInstalled,
        );
        assert!(s.is_partial());
        assert_eq!(s.cpu_millicores(), None);
        assert_eq!(s.cpu.missing(), Some(MissingReason::NotInstalled));
        assert_eq!(s.memory.missing(), Some(MissingReason::NotInstalled));
    }

    #[test]
    fn serde_round_trip_every_subject() {
        let subjects = [
            MetricsSubject::Node {
                node: "worker-1".into(),
            },
            MetricsSubject::Pod { pod: pod() },
            MetricsSubject::Container {
                pod: pod(),
                container: "app".into(),
            },
        ];
        for subject in subjects {
            // Window present.
            let with = MetricsSample::new(
                subject.clone(),
                ts(),
                Some(SignedDuration::from_secs(30)),
                5,
                6,
            );
            let json = serde_json::to_value(&with).unwrap();
            assert_eq!(serde_json::from_value::<MetricsSample>(json).unwrap(), with);

            // Window absent, readings partly missing.
            let mut without = MetricsSample::missing(subject, ts(), MissingReason::Forbidden);
            without.cpu = Reading::Value(1);
            let json = serde_json::to_value(&without).unwrap();
            assert!(json.get("window").is_none());
            assert_eq!(json["memory"], serde_json::json!({"missing": "forbidden"}));
            assert_eq!(json["cpu"], serde_json::json!({"value": 1}));
            assert_eq!(
                serde_json::from_value::<MetricsSample>(json).unwrap(),
                without
            );
        }
    }

    #[test]
    fn subject_wire_form_is_tagged() {
        let json = serde_json::to_value(MetricsSubject::Node { node: "n".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "node", "node": "n"}));
    }
}
