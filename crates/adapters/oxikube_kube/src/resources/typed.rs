//! `Api<K>` fast path for the core kinds.
//!
//! [`select`] picks a typed [`KindApi`] when the discovered kind is one of the bundled
//! `k8s-openapi` types and `ResourcesConfig::access_path` is [`Typed`](super::AccessPath::Typed).
//! The output is the type's own serialisation, which the kind golden test compares with the
//! dynamic path field for field.

use std::fmt::Debug;

use async_trait::async_trait;
use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, Secret, Service,
    ServiceAccount,
};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::ListParams;
use kube::core::ApiResource;
use kube::{Api, Client, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::backend::{KindApi, RawPage};

/// A [`KindApi`] over `Api<K>`.
struct Typed<K>(Api<K>)
where
    K: Resource + Clone + DeserializeOwned + Debug;

fn to_json<K: Resource + Serialize>(mut object: K, strip: bool) -> Value {
    if strip {
        object.meta_mut().managed_fields = None;
    }
    // Generated types serialise infallibly; fall through to Null (rejected downstream as
    // not-an-object) rather than panic.
    serde_json::to_value(&object).unwrap_or(Value::Null)
}

#[async_trait]
impl<K> KindApi for Typed<K>
where
    K: Resource + Clone + DeserializeOwned + Serialize + Debug + Send + Sync + 'static,
    K::DynamicType: Default,
{
    async fn list(&self, params: &ListParams, strip: bool) -> kube::Result<RawPage> {
        let list = self.0.list(params).await?;
        Ok(RawPage::from_list(list, |item| to_json(item, strip)))
    }

    async fn get_opt(&self, name: &str, strip: bool) -> kube::Result<Option<Value>> {
        Ok(self.0.get_opt(name).await?.map(|item| to_json(item, strip)))
    }
}

/// The typed API for the discovered kind, or `None` when it has no bundled type.
///
/// `namespace` is `None` for cluster-scoped kinds and for all-namespaces lists.
pub(super) fn select(
    client: &Client,
    resource: &ApiResource,
    namespace: Option<&str>,
) -> Option<Box<dyn KindApi>> {
    macro_rules! namespaced {
        ($ty:ty) => {
            Some(Box::new(Typed(namespaced_api::<$ty>(client, namespace))))
        };
    }
    macro_rules! cluster {
        ($ty:ty) => {
            Some(Box::new(Typed(Api::<$ty>::all(client.clone()))))
        };
    }
    match (
        resource.group.as_str(),
        resource.version.as_str(),
        resource.kind.as_str(),
    ) {
        ("", "v1", "Pod") => namespaced!(Pod),
        ("", "v1", "Service") => namespaced!(Service),
        ("", "v1", "ConfigMap") => namespaced!(ConfigMap),
        ("", "v1", "Secret") => namespaced!(Secret),
        ("", "v1", "ServiceAccount") => namespaced!(ServiceAccount),
        ("", "v1", "PersistentVolumeClaim") => namespaced!(PersistentVolumeClaim),
        ("", "v1", "Node") => cluster!(Node),
        ("", "v1", "Namespace") => cluster!(Namespace),
        ("", "v1", "PersistentVolume") => cluster!(PersistentVolume),
        ("apps", "v1", "Deployment") => namespaced!(Deployment),
        ("apps", "v1", "StatefulSet") => namespaced!(StatefulSet),
        ("apps", "v1", "DaemonSet") => namespaced!(DaemonSet),
        ("apps", "v1", "ReplicaSet") => namespaced!(ReplicaSet),
        ("batch", "v1", "Job") => namespaced!(Job),
        ("batch", "v1", "CronJob") => namespaced!(CronJob),
        ("networking.k8s.io", "v1", "Ingress") => namespaced!(Ingress),
        _ => None,
    }
}

fn namespaced_api<K>(client: &Client, namespace: Option<&str>) -> Api<K>
where
    K: Resource<Scope = NamespaceResourceScope>,
    K::DynamicType: Default,
{
    match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    }
}
