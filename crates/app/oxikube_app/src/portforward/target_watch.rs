//! What a forward watches: its pod, or its Service and the Service's EndpointSlices, merged into
//! one stream ([`TargetWatch`]) and folded into the [`TargetView`] the resolver reads.

use futures::stream::{BoxStream, StreamExt as _};
use oxikube_domain::Resource;
use oxikube_domain::portforward::ForwardSpec;
use oxikube_domain::session::WatchScope;
use oxikube_domain::view::SERVICE_NAME_LABEL;

use super::backend::{ForwardBackend, ObjectWatch};
use super::resolve::TargetView;
use crate::network::ServiceBackends;
use crate::store::{LabelSelector, LabelTerm, StoreFilter, StoreQuery};

/// Which feed a watch item comes from.
#[derive(Debug, Clone, Copy)]
pub(super) enum Part {
    Pod,
    Service,
    Slices,
}

/// The merged watch of a forward's target.
pub(super) struct TargetWatch(pub(super) BoxStream<'static, (Part, Result<Vec<Resource>, String>)>);

/// Watches what `spec`'s target needs: the pod, or the Service and its EndpointSlices.
pub(super) fn watch_target(
    backend: &dyn ForwardBackend,
    spec: &ForwardSpec,
) -> Option<TargetWatch> {
    let target = &spec.target;
    let cluster = &target.cluster;
    let namespace = target.namespace.as_deref()?.to_owned();
    let scope = WatchScope::Namespaces(vec![namespace]);
    let by_name = StoreFilter {
        name: Some(target.name.to_string()),
        ..StoreFilter::default()
    };
    let tag = |part: Part, watch: ObjectWatch| watch.map(move |item| (part, item));
    if spec.targets_service() {
        let service = StoreQuery::new(target.gvk.clone(), scope.clone()).with_filter(by_name);
        // A client-side label filter, not a server-side selector: the store keys feeds by
        // selector, so every Service forwarded in a namespace shares that namespace's one
        // EndpointSlice feed instead of opening a feed (and a watch-budget slot) each.
        let slices = StoreQuery::new(ServiceBackends::slice_gvk(), scope).with_filter(
            StoreFilter::labels(LabelSelector::from_terms(vec![LabelTerm::Eq(
                SERVICE_NAME_LABEL.to_owned(),
                target.name.to_string(),
            )])),
        );
        let service = tag(Part::Service, backend.watch(cluster, service)?);
        let slices = tag(Part::Slices, backend.watch(cluster, slices)?);
        Some(TargetWatch(
            futures::stream::select(service, slices).boxed(),
        ))
    } else {
        let pod = StoreQuery::new(target.gvk.clone(), scope).with_filter(by_name);
        Some(TargetWatch(
            tag(Part::Pod, backend.watch(cluster, pod)?).boxed(),
        ))
    }
}

pub(super) fn view_apply(view: &mut TargetView, part: Part, mut objects: Vec<Resource>) {
    match part {
        Part::Pod => view.pod = Some(objects.pop()),
        Part::Service => view.service = Some(objects.pop()),
        Part::Slices => view.slices = Some(objects),
    }
}
