//! The `metrics.k8s.io` list calls: one request per list, decoded by `k8s-metrics` with a thin
//! fallback.
//!
//! `metrics.k8s.io` serves no pagination or watch, so a list is a single request that returns
//! every object in scope (roughly 300 to 400 bytes per pod). The primary decoder is the `k8s-metrics`
//! crate's `NodeMetrics` / `PodMetrics`. If the server's payload does not decode into them (the
//! crate is single-maintainer and strict: one unreadable `window` fails the whole list), the
//! same endpoint is read again as `Api<DynamicObject>` and decoded leniently by
//! [`raw`](super::raw). The second request happens only after a decode failure.

use std::fmt::Debug;

use k8s_metrics::v1beta1::{NodeMetrics, PodMetrics};
use kube::api::ListParams;
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::{Api, Client};
use serde::de::DeserializeOwned;
use tracing::warn;

use super::error::Stop;
use super::raw::{RawNode, RawPod};

const GROUP: &str = "metrics.k8s.io";
const VERSION: &str = "v1beta1";

/// `Api<DynamicObject>` handle for the fallback decoder, in `namespace` or cluster-wide.
fn dynamic_api(
    client: &Client,
    kind: &str,
    plural: &str,
    namespace: Option<&str>,
) -> Api<DynamicObject> {
    let resource =
        ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(GROUP, VERSION, kind), plural);
    match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &resource),
        None => Api::all_with(client.clone(), &resource),
    }
}

/// Lists through the typed `k8s-metrics` decoder; only when the body does not decode, lists
/// again through `fallback`.
async fn list_with_fallback<K, R>(
    kind: &str,
    typed: Api<K>,
    fallback: impl FnOnce() -> Api<DynamicObject>,
    from_typed: fn(K) -> R,
    from_dynamic: fn(DynamicObject) -> R,
) -> Result<Vec<R>, Stop>
where
    K: Clone + DeserializeOwned + Debug,
{
    let params = ListParams::default();
    match typed.list(&params).await {
        Ok(list) => Ok(list.items.into_iter().map(from_typed).collect()),
        Err(kube::Error::SerdeError(_)) => {
            warn!("{kind} did not decode with k8s-metrics; reading with the fallback types");
            let list = fallback().list(&params).await?;
            Ok(list.items.into_iter().map(from_dynamic).collect())
        }
        Err(err) => Err(err.into()),
    }
}

/// Every node's metrics.
pub(super) async fn nodes(client: &Client) -> Result<Vec<RawNode>, Stop> {
    list_with_fallback(
        "NodeMetrics",
        Api::<NodeMetrics>::all(client.clone()),
        || dynamic_api(client, "NodeMetrics", "nodes", None),
        RawNode::from,
        RawNode::from_dynamic,
    )
    .await
}

/// Every pod's metrics in `namespace`, or in all namespaces for `None`.
pub(super) async fn pods(client: &Client, namespace: Option<&str>) -> Result<Vec<RawPod>, Stop> {
    let typed = match namespace {
        Some(ns) => Api::<PodMetrics>::namespaced(client.clone(), ns),
        None => Api::<PodMetrics>::all(client.clone()),
    };
    list_with_fallback(
        "PodMetrics",
        typed,
        || dynamic_api(client, "PodMetrics", "pods", namespace),
        RawPod::from,
        RawPod::from_dynamic,
    )
    .await
}
