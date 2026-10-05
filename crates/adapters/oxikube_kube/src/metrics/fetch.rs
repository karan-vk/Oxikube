//! The `metrics.k8s.io` list calls: one request per list, decoded by `k8s-metrics` with a thin
//! fallback.
//!
//! `metrics.k8s.io` serves no pagination or watch, so a list is a single request that returns
//! every object in scope (roughly 300 to 400 bytes per pod). The primary decoder is the `k8s-metrics`
//! crate's `NodeMetrics` / `PodMetrics`. If the server's payload does not decode into them (the
//! crate is single-maintainer and strict: one unreadable `window` fails the whole list), the
//! same endpoint is read again as `Api<DynamicObject>` and decoded leniently by
//! [`raw`](super::raw). The second request happens only after a decode failure.

use k8s_metrics::v1beta1::{NodeMetrics, PodMetrics};
use kube::api::ListParams;
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::{Api, Client};
use tracing::warn;

use super::raw::{RawNode, RawPod};

const GROUP: &str = "metrics.k8s.io";
const VERSION: &str = "v1beta1";

/// `Api<DynamicObject>` handle for the fallback decoder.
fn dynamic_resource(kind: &str, plural: &str) -> ApiResource {
    ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(GROUP, VERSION, kind), plural)
}

/// Whether the list failed because the body did not decode, which is what the fallback is for.
fn is_decode_failure(err: &kube::Error) -> bool {
    matches!(err, kube::Error::SerdeError(_))
}

/// Every node's metrics.
pub(super) async fn nodes(client: &Client) -> kube::Result<Vec<RawNode>> {
    let params = ListParams::default();
    let typed: Api<NodeMetrics> = Api::all(client.clone());
    match typed.list(&params).await {
        Ok(list) => Ok(list.items.into_iter().map(RawNode::from).collect()),
        Err(err) if is_decode_failure(&err) => {
            warn!("NodeMetrics did not decode with k8s-metrics; reading with the fallback types");
            let api: Api<DynamicObject> =
                Api::all_with(client.clone(), &dynamic_resource("NodeMetrics", "nodes"));
            let list = api.list(&params).await?;
            Ok(list.items.into_iter().map(RawNode::from_dynamic).collect())
        }
        Err(err) => Err(err),
    }
}

/// Every pod's metrics in `namespace`, or in all namespaces for `None`.
pub(super) async fn pods(client: &Client, namespace: Option<&str>) -> kube::Result<Vec<RawPod>> {
    let params = ListParams::default();
    let typed: Api<PodMetrics> = match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };
    match typed.list(&params).await {
        Ok(list) => Ok(list.items.into_iter().map(RawPod::from).collect()),
        Err(err) if is_decode_failure(&err) => {
            warn!("PodMetrics did not decode with k8s-metrics; reading with the fallback types");
            let resource = dynamic_resource("PodMetrics", "pods");
            let api: Api<DynamicObject> = match namespace {
                Some(ns) => Api::namespaced_with(client.clone(), ns, &resource),
                None => Api::all_with(client.clone(), &resource),
            };
            let list = api.list(&params).await?;
            Ok(list.items.into_iter().map(RawPod::from_dynamic).collect())
        }
        Err(err) => Err(err),
    }
}
