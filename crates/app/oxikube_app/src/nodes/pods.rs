//! [`list_node_pods`]: the pods on one node, asked of the API server.

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::{Demand, OxiResult, PodPlacement, Resource};
use oxikube_ports::{ListOptions, ResourceReader};

/// The pods on a node and what they take of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodePods {
    /// The summed demand of the pods that are not finished.
    pub demand: Demand,
    /// The pods that are not finished, by namespace and name.
    pub pods: Vec<Arc<Resource>>,
}

/// The pods on `node`, with `spec.nodeName=<node>` as a field selector so the server returns only
/// them (finished pods are excluded server-side too, as `kubectl describe node` does). Follows
/// the continue tokens, so a busy node is complete.
///
/// This is the fallback behind the node detail; with the Pod feed active the detail reads the
/// store's index instead and sends nothing.
///
/// # Errors
///
/// The API server's error (`Forbidden` for a user who may not list pods).
pub async fn list_node_pods(reader: &dyn ResourceReader, node: &str) -> OxiResult<NodePods> {
    let kind = Gvk::new("", "v1", "Pod");
    let mut options = ListOptions::default().fields(format!(
        "spec.nodeName={node},status.phase!=Succeeded,status.phase!=Failed"
    ));
    let mut found = NodePods::default();
    loop {
        let page = reader.list(&kind, None, &options).await?;
        for pod in page.items {
            // The selector already left finished pods out; the placement check also drops a
            // server that ignored it.
            if let Some(placement) = PodPlacement::of_pod(&pod)
                && &*placement.node == node
            {
                found.demand += placement.demand;
                found.pods.push(Arc::new(pod));
            }
        }
        match page.continue_token {
            Some(token) => options = options.continue_from(token),
            None => break,
        }
    }
    found.pods.sort_by(|a, b| {
        (a.meta.namespace.as_deref(), &*a.meta.name)
            .cmp(&(b.meta.namespace.as_deref(), &*b.meta.name))
    });
    Ok(found)
}
