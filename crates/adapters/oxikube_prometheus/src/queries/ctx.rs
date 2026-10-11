//! [`Ctx`]: what the builders of one query share.

use std::fmt::Display;

use oxikube_domain::{MetricQueryError, MetricRequest, MetricTarget};

use super::profile::Profile;
use super::selector::{Matcher, Selector};

/// Pods that hold their requests: pending or running ones, as kube-prometheus' own rules count.
pub(super) const LIVE_PHASES: &str = "Pending|Running";

/// The `rate()` window, the stack's profile and the request being built.
pub(super) struct Ctx<'a> {
    pub profile: &'a Profile,
    pub request: &'a MetricRequest,
    /// The window as PromQL writes it (`2m`).
    pub window: String,
}

impl<'a> Ctx<'a> {
    pub(super) fn target(&self) -> &'a MetricTarget {
        &self.request.target
    }

    /// A selector of `metric` with `matchers` plus the request's extra matchers.
    pub(super) fn select(&self, metric: &'static str, mut matchers: Vec<Matcher>) -> Selector {
        matchers.extend(
            self.request
                .extra
                .iter()
                .map(|label| Matcher::eq(label.name(), label.value())),
        );
        Selector::new(metric, matchers)
    }

    /// `sum(inner)`, or `sum by (pod|container) (inner)` for the scopes whose result has a
    /// series per pod or container.
    pub(super) fn sum(&self, inner: impl Display) -> String {
        match self.target().series_label() {
            Some(label) => format!("sum by ({label}) ({inner})"),
            None => format!("sum({inner})"),
        }
    }

    /// `rate(selector[window])`.
    pub(super) fn rate(&self, selector: &Selector) -> String {
        format!("rate({selector}[{}])", self.window)
    }

    /// The matchers that pick the target's objects in cAdvisor and kube-state-metrics series
    /// (both use `namespace`, `pod` and `container`; kube-state-metrics adds `node`).
    pub(super) fn workload_matchers(&self) -> Vec<Matcher> {
        match self.target() {
            MetricTarget::Cluster | MetricTarget::Pvc { .. } | MetricTarget::Ingress { .. } => {
                Vec::new()
            }
            MetricTarget::Node(node) => vec![Matcher::eq("node", node)],
            MetricTarget::Namespace(namespace) => vec![Matcher::eq("namespace", namespace)],
            MetricTarget::Pod { namespace, pods } => {
                vec![
                    Matcher::eq("namespace", namespace),
                    Matcher::one_of("pod", pods.names()),
                ]
            }
            MetricTarget::Container {
                namespace,
                pod,
                container,
            } => {
                let mut matchers =
                    vec![Matcher::eq("namespace", namespace), Matcher::eq("pod", pod)];
                if let Some(container) = container {
                    matchers.push(Matcher::eq("container", container));
                }
                matchers
            }
        }
    }

    /// [`workload_matchers`](Self::workload_matchers) for a cAdvisor series, which also has
    /// series for the pod's cgroup (`container=""`) and the pause container: drop both unless
    /// one container was asked for by name.
    pub(super) fn cadvisor_matchers(&self) -> Vec<Matcher> {
        let mut matchers = self.workload_matchers();
        if !matches!(
            self.target(),
            MetricTarget::Container {
                container: Some(_),
                ..
            }
        ) {
            matchers.push(Matcher::ne("container", ""));
            matchers.push(Matcher::ne("container", "POD"));
        }
        matchers
    }

    /// The error for a combination the builders do not cover.
    pub(super) fn unsupported(&self) -> MetricQueryError {
        MetricQueryError::Unsupported {
            scope: self.request.scope(),
            kind: self.request.kind,
        }
    }
}
