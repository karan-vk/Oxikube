//! The producer side of an editor's live object, run on the Kubernetes runtime
//! (`oxikube_runtime::spawn_kube`): finding the object a manifest names, and watching it.

use std::sync::Arc;

use futures::channel::mpsc::Sender;
use futures::{SinkExt as _, StreamExt as _};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{ErrorKind, OxiResult, Resource};
use oxikube_ports::{Delta, DeltaBatch, ResourceReader, WatchOptions};

use crate::diff::ObjectId;

/// The latest state of the watched object.
#[derive(Clone, Debug)]
pub(crate) enum Watched {
    /// It exists, as this.
    Object(Resource),
    /// It does not exist (deleted, or not created yet).
    Gone,
    /// The watch failed; the message names no object content.
    Failed(String),
}

/// Watches the one object `target` names (a `metadata.name` field selector, in its namespace)
/// and sends its latest state after every batch, until the stream ends or the receiver is gone.
/// Dropping the future (the editor's task) closes the watch.
pub(crate) async fn watch_object(
    reader: Arc<dyn ResourceReader>,
    target: ResourceRef,
    mut tx: Sender<Watched>,
) {
    let options = WatchOptions::default().fields(format!("metadata.name={}", target.name));
    let feed = reader
        .watch(&target.gvk, target.namespace.as_deref(), &options)
        .await;
    let mut feed = match feed {
        Ok(feed) => feed,
        Err(error) => {
            let _ = tx.send(Watched::Failed(error.to_string())).await;
            return;
        }
    };
    while let Some(batch) = feed.next().await {
        let state = match batch {
            Ok(batch) => latest(batch, &target),
            Err(error) => Some(Watched::Failed(error.to_string())),
        };
        if let Some(state) = state
            && tx.send(state).await.is_err()
        {
            return;
        }
    }
}

/// The object's state after `batch`, if the batch says anything about it.
fn latest(batch: DeltaBatch, target: &ResourceRef) -> Option<Watched> {
    let is_target = |r: &Resource| names(target, r);
    let mut state = None;
    for delta in batch.deltas {
        match delta {
            Delta::Restarted(objects) => {
                state = Some(
                    objects
                        .into_iter()
                        .find(|r| is_target(r))
                        .map_or(Watched::Gone, Watched::Object),
                );
            }
            Delta::Applied(object) if is_target(&object) => state = Some(Watched::Object(object)),
            Delta::Deleted(object) if is_target(&object) => state = Some(Watched::Gone),
            Delta::Applied(_) | Delta::Deleted(_) => {}
        }
    }
    state
}

/// Whether `object` is the one `target` names. A target without a namespace (a manifest that
/// names none) is what `kubectl apply` reads it as: the cluster-scoped object, or for a
/// namespaced kind the one in [`DEFAULT_NAMESPACE`], never a same-named object in another
/// namespace (its watch is cluster-wide: the kind's scope is not known here).
fn names(target: &ResourceRef, object: &Resource) -> bool {
    object.name() == &*target.name
        && match target.namespace.as_deref() {
            Some(ns) => object.namespace() == Some(ns),
            None => object.namespace().is_none_or(|ns| ns == DEFAULT_NAMESPACE),
        }
}

/// The namespace a manifest without one goes to, as `kubectl` defaults it.
const DEFAULT_NAMESPACE: &str = "default";

/// The object `id` names, read once. A manifest without a namespace is looked up as written
/// (cluster-scoped), then in [`DEFAULT_NAMESPACE`], as `kubectl` does for a namespaced kind. `Ok(None)`
/// when it does not exist.
pub(crate) async fn find_object(
    reader: Arc<dyn ResourceReader>,
    id: ObjectId,
) -> OxiResult<Option<Resource>> {
    let found = reader
        .get_opt(&id.gvk, id.namespace.as_deref(), &id.name)
        .await;
    match (found, &id.namespace) {
        (Ok(None), None) => {
            reader
                .get_opt(&id.gvk, Some(DEFAULT_NAMESPACE), &id.name)
                .await
        }
        (Err(error), None) if error.kind() == ErrorKind::NotFound => {
            reader
                .get_opt(&id.gvk, Some(DEFAULT_NAMESPACE), &id.name)
                .await
        }
        (found, _) => found,
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::{ClusterId, ContextName};
    use oxikube_ports::Delta;
    use serde_json::json;

    use super::*;

    fn config_map(namespace: &str) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {"name": "web", "namespace": namespace}
        }))
        .unwrap()
    }

    fn target(namespace: Option<&str>) -> ResourceRef {
        let object = config_map("default");
        let cluster = ClusterId::new("/kubeconfig", &ContextName::new("kind"));
        ResourceRef::new(
            cluster,
            object.kind.clone(),
            namespace.map(Arc::from),
            object.name(),
        )
    }

    fn restarted(objects: Vec<Resource>) -> DeltaBatch {
        DeltaBatch::from_deltas(vec![Delta::Restarted(objects)])
    }

    #[test]
    fn a_target_without_a_namespace_is_never_a_same_named_object_elsewhere() {
        let state = latest(restarted(vec![config_map("shop")]), &target(None));
        assert!(matches!(state, Some(Watched::Gone)), "{state:?}");
        let applied = DeltaBatch::from_deltas(vec![Delta::Applied(config_map("shop"))]);
        assert!(latest(applied, &target(None)).is_none());
    }

    #[test]
    fn a_target_without_a_namespace_is_the_one_in_default() {
        let objects = vec![config_map("shop"), config_map("default")];
        let state = latest(restarted(objects), &target(None));
        let Some(Watched::Object(object)) = state else {
            panic!("{state:?}");
        };
        assert_eq!(object.namespace(), Some("default"));
    }

    #[test]
    fn a_target_in_a_namespace_is_only_that_one() {
        let objects = vec![config_map("default"), config_map("shop")];
        let Some(Watched::Object(object)) = latest(restarted(objects), &target(Some("shop")))
        else {
            panic!("the object in shop");
        };
        assert_eq!(object.namespace(), Some("shop"));
    }
}
