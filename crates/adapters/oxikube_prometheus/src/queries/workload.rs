//! Namespace, pod and container scopes: usage from cAdvisor, requests and limits from
//! kube-state-metrics (and, for cluster and node scopes, the requests and limits too).

use oxikube_domain::{MetricKind, MetricQueryError, MetricScope};

use super::ctx::{Ctx, LIVE_PHASES};
use super::selector::Matcher;

pub(super) fn usage(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    let inner = match ctx.request.kind {
        MetricKind::CpuUsage => {
            ctx.rate(&ctx.select("container_cpu_usage_seconds_total", ctx.cadvisor_matchers()))
        }
        MetricKind::MemoryUsage => ctx
            .select(
                "container_memory_working_set_bytes",
                ctx.cadvisor_matchers(),
            )
            .to_string(),
        _ => return Err(ctx.unsupported()),
    };
    Ok(ctx.sum(inner))
}

/// Requests or limits of one resource. The aggregate scopes count only pods that are pending or
/// running (a finished Job's pod keeps its series but holds nothing); a pod or container asked
/// for by name is shown as it is.
pub(super) fn reference(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    let (metric, resource) = match ctx.request.kind {
        MetricKind::CpuRequests => ("kube_pod_container_resource_requests", "cpu"),
        MetricKind::CpuLimits => ("kube_pod_container_resource_limits", "cpu"),
        MetricKind::MemoryRequests => ("kube_pod_container_resource_requests", "memory"),
        MetricKind::MemoryLimits => ("kube_pod_container_resource_limits", "memory"),
        _ => return Err(ctx.unsupported()),
    };
    let target_matchers = ctx.workload_matchers();
    let mut matchers = vec![Matcher::eq("resource", resource)];
    matchers.extend(target_matchers.iter().cloned());
    let series = ctx.select(metric, matchers);
    match ctx.request.scope() {
        MetricScope::Pod | MetricScope::Container => Ok(ctx.sum(series)),
        MetricScope::Cluster | MetricScope::Node | MetricScope::Namespace => {
            let mut live = vec![Matcher::re("phase", LIVE_PHASES)];
            if let Some(namespace) = target_matchers
                .into_iter()
                .find(|m| m.is_label("namespace"))
            {
                live.push(namespace);
            }
            let phase = ctx.select("kube_pod_status_phase", live);
            Ok(format!(
                "sum({series} * on (namespace, pod) group_left() ({phase} == 1))"
            ))
        }
        MetricScope::Pvc | MetricScope::Ingress => Err(ctx.unsupported()),
    }
}
