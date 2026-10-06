//! [`NativeDescribe`]: deskribe over the connection's kube client.

use std::sync::Arc;

use async_trait::async_trait;
use kube::Client;
use kube::api::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DescribeOutput, DescribePort, DescribeSource, DiscoveryPort};

use crate::errors::classify;

/// Renders `kubectl describe`-style text in process with deskribe.
///
/// deskribe reads the object fresh, its related objects and its events through `client`, then
/// renders offline. The kind's plural (which a `ResourceRef` does not carry) comes from
/// discovery, so custom resources work like core kinds.
pub struct NativeDescribe {
    client: Client,
    discovery: Arc<dyn DiscoveryPort>,
}

impl std::fmt::Debug for NativeDescribe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeDescribe").finish_non_exhaustive()
    }
}

impl NativeDescribe {
    /// A describer over `client`, resolving kinds through `discovery`.
    pub fn new(client: Client, discovery: Arc<dyn DiscoveryPort>) -> Self {
        Self { client, discovery }
    }

    /// The REST description of `target`'s kind, from discovery. `Unsupported` when the cluster
    /// does not serve the kind.
    async fn api_resource(&self, target: &ResourceRef) -> OxiResult<ApiResource> {
        let kind = self.discovery.resolve(&target.gvk).await?.ok_or_else(|| {
            OxiError::unsupported(format!("the cluster does not serve {}", target.gvk))
        })?;
        let gvk = GroupVersionKind::gvk(&kind.gvk.group, &kind.gvk.version, &kind.gvk.kind);
        Ok(ApiResource::from_gvk_with_plural(&gvk, &kind.plural))
    }
}

#[async_trait]
impl DescribePort for NativeDescribe {
    async fn describe(&self, target: &ResourceRef) -> OxiResult<DescribeOutput> {
        let resource = self.api_resource(target).await?;
        if !deskribe::supports(&resource) {
            return Err(OxiError::unsupported(format!(
                "native describe does not cover {}",
                target.gvk.kind
            )));
        }
        let mut selected = DynamicObject::new(&target.name, &resource);
        selected.metadata.namespace = target.namespace.as_deref().map(str::to_owned);
        let (_, text) = deskribe::fetch(self.client.clone(), &resource, &selected)
            .await
            .map_err(|message| classify(&message))?;
        Ok(DescribeOutput {
            text,
            source: DescribeSource::Native,
        })
    }
}
