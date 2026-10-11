//! [`MetricsProviderKind`]: which Prometheus-compatible stack serves a cluster's history.
//!
//! Detection (E13-S05) classifies what it finds; the query catalogue (E13-S06) picks label names
//! and query variants by kind; the per-cluster `prometheus.kind` setting names one by hand.
//! The kinds are the stacks of ADR 0011: kube-prometheus / the Prometheus Operator, the Lens
//! metrics stack, VictoriaMetrics, Mimir and OpenShift's Thanos querier, plus a plain
//! Prometheus for anything else that speaks the same API.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A family of Prometheus-compatible metrics backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricsProviderKind {
    /// kube-prometheus-stack or the Prometheus Operator (`prometheus-operated`).
    KubePrometheus,
    /// The metrics stack Lens installs (`lens-metrics/prometheus`).
    Lens,
    /// VictoriaMetrics (`vmsingle`, `vmselect`).
    VictoriaMetrics,
    /// Grafana Mimir (`mimir-query-frontend`).
    Mimir,
    /// OpenShift's monitoring stack (`thanos-querier` in `openshift-monitoring`).
    #[serde(rename = "openshift")]
    OpenShift,
    /// A plain Prometheus-compatible API: a stand-alone Prometheus, Thanos, Cortex, or any
    /// endpoint the user configured without naming a stack.
    Prometheus,
}

impl MetricsProviderKind {
    /// Every kind, in the order the settings schema lists them.
    pub const ALL: [MetricsProviderKind; 6] = [
        MetricsProviderKind::KubePrometheus,
        MetricsProviderKind::Lens,
        MetricsProviderKind::VictoriaMetrics,
        MetricsProviderKind::Mimir,
        MetricsProviderKind::OpenShift,
        MetricsProviderKind::Prometheus,
    ];

    /// The stable id used in settings files and the detection catalogue (`kube_prometheus`).
    pub const fn id(self) -> &'static str {
        match self {
            MetricsProviderKind::KubePrometheus => "kube_prometheus",
            MetricsProviderKind::Lens => "lens",
            MetricsProviderKind::VictoriaMetrics => "victoria_metrics",
            MetricsProviderKind::Mimir => "mimir",
            MetricsProviderKind::OpenShift => "openshift",
            MetricsProviderKind::Prometheus => "prometheus",
        }
    }

    /// The name shown to the user.
    pub const fn label(self) -> &'static str {
        match self {
            MetricsProviderKind::KubePrometheus => "kube-prometheus",
            MetricsProviderKind::Lens => "Lens metrics stack",
            MetricsProviderKind::VictoriaMetrics => "VictoriaMetrics",
            MetricsProviderKind::Mimir => "Mimir",
            MetricsProviderKind::OpenShift => "OpenShift monitoring",
            MetricsProviderKind::Prometheus => "Prometheus",
        }
    }
}

impl fmt::Display for MetricsProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A string that names no [`MetricsProviderKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProviderKind(pub String);

impl fmt::Display for UnknownProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let known = MetricsProviderKind::ALL
            .map(MetricsProviderKind::id)
            .join(", ");
        write!(
            f,
            "unknown metrics provider `{}` (expected one of: {known})",
            self.0
        )
    }
}

impl std::error::Error for UnknownProviderKind {}

impl FromStr for MetricsProviderKind {
    type Err = UnknownProviderKind;

    /// Reads an [`id`](Self::id); the older names of the operator stack (`operator`, `helm`) and
    /// the spelling `victoriametrics` are accepted too. Case and surrounding space are ignored.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let key = s.trim().to_ascii_lowercase().replace('-', "_");
        Ok(match key.as_str() {
            "kube_prometheus" | "operator" => MetricsProviderKind::KubePrometheus,
            "lens" => MetricsProviderKind::Lens,
            "victoria_metrics" | "victoriametrics" | "vm" => MetricsProviderKind::VictoriaMetrics,
            "mimir" => MetricsProviderKind::Mimir,
            "openshift" => MetricsProviderKind::OpenShift,
            "prometheus" | "helm" | "thanos" => MetricsProviderKind::Prometheus,
            _ => return Err(UnknownProviderKind(s.to_owned())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for kind in MetricsProviderKind::ALL {
            assert_eq!(kind.id().parse::<MetricsProviderKind>(), Ok(kind));
            assert!(!kind.label().is_empty());
        }
    }

    #[test]
    fn older_names_and_spellings_are_accepted() {
        assert_eq!("Operator".parse(), Ok(MetricsProviderKind::KubePrometheus));
        assert_eq!(
            " victoria-metrics ".parse(),
            Ok(MetricsProviderKind::VictoriaMetrics)
        );
        assert_eq!("helm".parse(), Ok(MetricsProviderKind::Prometheus));
    }

    #[test]
    fn an_unknown_name_lists_the_known_ones() {
        let error = "grafana".parse::<MetricsProviderKind>().unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains("grafana") && text.contains("kube_prometheus"),
            "{text}"
        );
    }

    #[test]
    fn serde_uses_the_ids() {
        for kind in MetricsProviderKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.id()));
            assert_eq!(
                serde_json::from_str::<MetricsProviderKind>(&json).unwrap(),
                kind
            );
        }
    }
}
