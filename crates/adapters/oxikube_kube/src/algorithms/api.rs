//! What the algorithms share: the kinds they touch and paginated listing over a `ResourcePort`.

use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourcePort};

pub(crate) fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

pub(crate) fn node_gvk() -> Gvk {
    Gvk::new("", "v1", "Node")
}

pub(crate) fn cronjob_gvk() -> Gvk {
    Gvk::new("batch", "v1", "CronJob")
}

pub(crate) fn job_gvk() -> Gvk {
    Gvk::new("batch", "v1", "Job")
}

pub(crate) fn deployment_gvk() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

pub(crate) fn replicaset_gvk() -> Gvk {
    Gvk::new("apps", "v1", "ReplicaSet")
}

/// Every object `options` selects, following continue tokens to the last page.
pub(crate) async fn list_all(
    port: &dyn ResourcePort,
    gvk: &Gvk,
    namespace: Option<&str>,
    mut options: ListOptions,
) -> OxiResult<Vec<Resource>> {
    let mut items = Vec::new();
    loop {
        let page = port.list(gvk, namespace, &options).await?;
        items.extend(page.items);
        match page.continue_token.filter(|token| !token.is_empty()) {
            Some(token) => options = options.continue_from(token),
            None => return Ok(items),
        }
    }
}
