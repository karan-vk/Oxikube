//! Finding a Deployment's ReplicaSets: the list is the adapter's, the reading of it is the
//! domain's ([`oxikube_domain::rollout`]).

use oxikube_domain::rollout::{label_selector, owned_replicasets};
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourcePort};

use crate::algorithms::api::{list_all, replicaset_gvk};

pub use oxikube_domain::rollout::REVISION_ANNOTATION;

/// The ReplicaSets the Deployment controls, each with its revision, oldest revision first.
/// ReplicaSets without a readable revision are not part of the history.
pub(super) async fn revisions(
    port: &dyn ResourcePort,
    deployment: &Resource,
) -> OxiResult<Vec<(i64, Resource)>> {
    let selector = label_selector(deployment)?;
    let options = ListOptions::default().labels(selector).limit(500);
    let replicasets = list_all(port, &replicaset_gvk(), deployment.namespace(), options).await?;
    Ok(owned_replicasets(deployment, replicasets))
}
