//! The requests every provider is asked for: one target per shape, every kind.

use oxikube_domain::{MetricKind, MetricRequest, MetricTarget, MetricsProviderKind, PodSelector};

/// A named target of each scope (and the shapes that change the query: a set of pods, every
/// container versus one).
pub fn targets() -> Vec<(&'static str, MetricTarget)> {
    vec![
        ("cluster", MetricTarget::Cluster),
        ("node worker-1", MetricTarget::Node("worker-1".into())),
        ("namespace shop", MetricTarget::Namespace("shop".into())),
        (
            "pod shop/web-0",
            MetricTarget::Pod {
                namespace: "shop".into(),
                pods: PodSelector::One("web-0".into()),
            },
        ),
        (
            "pods shop/web-7d9f-abcde,web-7d9f-fghij",
            MetricTarget::Pod {
                namespace: "shop".into(),
                pods: PodSelector::Any(vec!["web-7d9f-abcde".into(), "web-7d9f-fghij".into()]),
            },
        ),
        (
            "containers shop/web-0",
            MetricTarget::Container {
                namespace: "shop".into(),
                pod: "web-0".into(),
                container: None,
            },
        ),
        (
            "container shop/web-0/app",
            MetricTarget::Container {
                namespace: "shop".into(),
                pod: "web-0".into(),
                container: Some("app".into()),
            },
        ),
        (
            "pvc shop/data-web-0",
            MetricTarget::Pvc {
                namespace: "shop".into(),
                name: "data-web-0".into(),
            },
        ),
        (
            "ingress shop/web",
            MetricTarget::Ingress {
                namespace: "shop".into(),
                name: "web".into(),
            },
        ),
    ]
}

/// Every provider, in settings order.
pub fn providers() -> [MetricsProviderKind; 6] {
    MetricsProviderKind::ALL
}

/// Every (provider, target label, request) the catalogue is asked.
pub fn all() -> Vec<(MetricsProviderKind, &'static str, MetricRequest)> {
    let mut out = Vec::new();
    for provider in providers() {
        for (label, target) in targets() {
            for kind in MetricKind::ALL {
                out.push((provider, label, MetricRequest::new(target.clone(), kind)));
            }
        }
    }
    out
}
