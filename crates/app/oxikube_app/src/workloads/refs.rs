//! What a pod points at (E12-S03): its owner chain, its node and the ConfigMaps and Secrets its
//! environment names. All reads, on demand, one `get` per object (no new feed).

use std::sync::Arc;

use futures::future::join_all;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef, Scope};
use oxikube_domain::view::{EnvSources, PodDetail, SourceData, SourceKind, SourceState};
use oxikube_domain::{ErrorKind, OxiError, OxiResult, Resource};

use super::service::WorkloadService;

/// How many owners up the chain walks: Pod, ReplicaSet, Deployment is two; a Rollout or an
/// operator's custom resource a few more.
const MAX_OWNER_DEPTH: usize = 6;

impl WorkloadService {
    /// The owners of `pod`, nearest first: its ReplicaSet, then the Deployment that owns that
    /// (Pod → ReplicaSet → Deployment). At each level the controller is followed, else the first
    /// owner. The chain ends at an object nobody owns, at one that is gone (it is still listed),
    /// or after [`MAX_OWNER_DEPTH`] levels.
    ///
    /// # Errors
    ///
    /// The server's error for reading `pod` itself (`NotFound`, `Forbidden`); a missing or
    /// unreadable owner only ends the chain.
    pub async fn owner_chain(&self, pod: &ResourceRef) -> OxiResult<Vec<ResourceRef>> {
        let reader = self.reader(&pod.cluster)?;
        let mut object = reader.get(&pod.gvk, pod.namespace(), &pod.name).await?;
        let mut chain: Vec<ResourceRef> = Vec::new();
        while chain.len() < MAX_OWNER_DEPTH {
            let Some(target) = self.owner_ref(pod, &object).await else {
                break;
            };
            let next = reader
                .get_opt(&target.gvk, target.namespace(), &target.name)
                .await;
            chain.push(target);
            match next {
                Ok(Some(owner)) => object = owner,
                _ => break,
            }
        }
        Ok(chain)
    }

    /// The nearest owner of `pod` (what `shift-j` opens), `None` for a pod nobody owns.
    ///
    /// # Errors
    ///
    /// As [`owner_chain`](Self::owner_chain).
    pub async fn owner_of(&self, pod: &ResourceRef) -> OxiResult<Option<ResourceRef>> {
        let reader = self.reader(&pod.cluster)?;
        let live = reader.get(&pod.gvk, pod.namespace(), &pod.name).await?;
        Ok(self.owner_ref(pod, &live).await)
    }

    /// The controller of `object` (else its first owner), as a reference in `pod`'s cluster and,
    /// for a namespaced kind, namespace.
    async fn owner_ref(&self, pod: &ResourceRef, object: &Resource) -> Option<ResourceRef> {
        let owner = object
            .meta
            .controller_ref()
            .or_else(|| object.meta.owner_refs.first())?;
        let gvk = owner.gvk();
        let namespace = match self.scope_of(&pod.cluster, &gvk).await {
            Scope::Namespaced => pod.namespace().map(Arc::from),
            Scope::Cluster => None,
        };
        Some(ResourceRef::new(
            pod.cluster.clone(),
            gvk,
            namespace,
            owner.name.clone(),
        ))
    }

    /// The Node `pod` runs on (what `o` opens), `None` for a pod the scheduler has not placed.
    ///
    /// # Errors
    ///
    /// The server's error for reading `pod`.
    pub async fn node_of(&self, pod: &ResourceRef) -> OxiResult<Option<ResourceRef>> {
        let reader = self.reader(&pod.cluster)?;
        let live = reader.get(&pod.gvk, pod.namespace(), &pod.name).await?;
        Ok(oxikube_domain::view::node_of(&live).map(|node| {
            ResourceRef::cluster_scoped(pod.cluster.clone(), Gvk::new("", "v1", "Node"), node)
        }))
    }

    /// Looks up the ConfigMaps and Secrets the environment of `pod` (in `cluster`) names, each
    /// once (one `get` per object, concurrently), for
    /// [`ContainerDetail::env_rows`](oxikube_domain::view::ContainerDetail::env_rows). A missing
    /// object is
    /// [`SourceState::NotFound`], one the user may not read [`SourceState::Unavailable`]; neither
    /// fails the lookup. Secret values are decoded here and live in the returned value only.
    ///
    /// # Errors
    ///
    /// `Conflict` when the cluster is not connected.
    pub async fn env_sources(&self, cluster: &ClusterId, pod: &Resource) -> OxiResult<EnvSources> {
        let Ok(detail) = PodDetail::from_resource(pod) else {
            return Ok(EnvSources::default());
        };
        let refs = detail.source_refs();
        if refs.is_empty() {
            return Ok(EnvSources::default());
        }
        let reader = self.reader(cluster)?;
        let namespace = pod.namespace().map(str::to_owned);
        let lookups = refs.iter().map(|(kind, name)| {
            let reader = reader.clone();
            let namespace = namespace.clone();
            async move {
                let gvk = Gvk::new("", "v1", kind.as_str());
                match reader.get_opt(&gvk, namespace.as_deref(), name).await {
                    Ok(Some(object)) => SourceState::Found(match kind {
                        SourceKind::ConfigMap => SourceData::from_config_map(&object),
                        SourceKind::Secret => SourceData::from_secret(&object),
                    }),
                    Ok(None) => SourceState::NotFound,
                    Err(error) => SourceState::Unavailable(unavailable(&error)),
                }
            }
        });
        let states = join_all(lookups).await;
        let mut sources = EnvSources::default();
        for ((kind, name), state) in refs.into_iter().zip(states) {
            sources.insert(kind, name, state);
        }
        Ok(sources)
    }
}

/// Why an object could not be read, in a few words (never its content).
fn unavailable(error: &OxiError) -> String {
    match error.kind() {
        ErrorKind::Forbidden => "you may not read it".to_owned(),
        _ => error.message().to_owned(),
    }
}
