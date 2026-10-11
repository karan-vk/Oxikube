//! Disk: node filesystems from node-exporter, volumes from the kubelet, writable layers from
//! cAdvisor.

use oxikube_domain::{MetricKind, MetricQueryError, MetricScope, MetricTarget};

use super::ctx::Ctx;
use super::host;
use super::selector::Matcher;

pub(super) fn build(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    match ctx.request.scope() {
        MetricScope::Cluster | MetricScope::Node => host::build(ctx),
        MetricScope::Namespace | MetricScope::Pvc => volumes(ctx),
        MetricScope::Pod | MetricScope::Container => writable_layers(ctx),
        MetricScope::Ingress => Err(ctx.unsupported()),
    }
}

/// `kubelet_volume_stats_*` of the claims in a namespace, or of one claim.
fn volumes(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    let metric = match ctx.request.kind {
        MetricKind::DiskUsage => "kubelet_volume_stats_used_bytes",
        MetricKind::DiskCapacity => "kubelet_volume_stats_capacity_bytes",
        _ => return Err(ctx.unsupported()),
    };
    let mut matchers = Vec::new();
    match ctx.target() {
        MetricTarget::Namespace(namespace) => matchers.push(Matcher::eq("namespace", namespace)),
        MetricTarget::Pvc { namespace, name } => {
            matchers.push(Matcher::eq("namespace", namespace));
            matchers.push(Matcher::eq("persistentvolumeclaim", name));
        }
        _ => return Err(ctx.unsupported()),
    }
    Ok(ctx.sum(ctx.select(metric, matchers)))
}

/// The ephemeral storage a pod's containers use (`container_fs_usage_bytes`).
fn writable_layers(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    if ctx.request.kind != MetricKind::DiskUsage {
        return Err(ctx.unsupported());
    }
    let series = ctx.select("container_fs_usage_bytes", ctx.cadvisor_matchers());
    Ok(ctx.sum(series))
}
