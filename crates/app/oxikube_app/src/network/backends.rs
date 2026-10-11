//! [`ServiceBackends`]: the store queries behind a Service's detail.

use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_domain::view::{NoSelector, SERVICE_NAME_LABEL, ServiceSummary};

use crate::store::{LabelSelector, LabelTerm, StoreQuery};

/// The pods of a Service, or why there is no query for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodsPlan {
    /// Subscribe to this query: the Service's selector, as a server-side label selector.
    Query(StoreQuery),
    /// The Service selects no pods; the detail says why.
    None(NoSelector),
}

/// What a Service's detail subscribes to, both scoped to the Service's namespace and both
/// filtered by the server (`labelSelector`), so neither lists the namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceBackends {
    /// The EndpointSlices of the Service: `kubernetes.io/service-name=<name>`.
    pub slices: StoreQuery,
    /// The Service's pods.
    pub pods: PodsPlan,
}

impl ServiceBackends {
    /// The `discovery.k8s.io/v1` EndpointSlice kind.
    pub fn slice_gvk() -> Gvk {
        Gvk::new("discovery.k8s.io", "v1", "EndpointSlice")
    }

    /// The core Pod kind.
    pub fn pod_gvk() -> Gvk {
        Gvk::new("", "v1", "Pod")
    }

    /// The queries for `service`.
    pub fn of(service: &ServiceSummary) -> Self {
        let scope = match &service.namespace {
            Some(namespace) => WatchScope::Namespaces(vec![namespace.to_string()]),
            None => WatchScope::Cluster,
        };
        let by_service = LabelSelector::from_terms(vec![LabelTerm::Eq(
            SERVICE_NAME_LABEL.to_owned(),
            service.name.to_string(),
        )]);
        let slices =
            StoreQuery::new(Self::slice_gvk(), scope.clone()).with_selector(Some(by_service));
        let pods = match service.no_selector() {
            Some(reason) => PodsPlan::None(reason),
            None => {
                let terms = service
                    .selector
                    .iter()
                    .map(|(k, v)| LabelTerm::Eq(k.to_string(), v.to_string()))
                    .collect();
                PodsPlan::Query(
                    StoreQuery::new(Self::pod_gvk(), scope)
                        .with_selector(Some(LabelSelector::from_terms(terms))),
                )
            }
        };
        Self { slices, pods }
    }
}
