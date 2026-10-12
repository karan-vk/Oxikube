//! Network traffic: pods and namespaces from cAdvisor, hosts from node-exporter, Ingresses from
//! their controller.
//!
//! cAdvisor counts network per pod (the container label is empty), so there is no per-container
//! traffic, and a host-network pod reports the node's interfaces.

use oxikube_domain::{MetricKind, MetricQueryError, MetricScope, MetricTarget};

use super::ctx::Ctx;
use super::host;
use super::profile::IngressSource;
use super::selector::{Matcher, regex_quote};

pub(super) fn build(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    match ctx.request.scope() {
        MetricScope::Cluster | MetricScope::Node => host::build(ctx),
        MetricScope::Namespace | MetricScope::Pod => pods(ctx),
        MetricScope::Ingress => ingress(ctx),
        MetricScope::Container | MetricScope::Pvc => Err(ctx.unsupported()),
    }
}

fn pods(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    let metric = match ctx.request.kind {
        MetricKind::NetworkRx => "container_network_receive_bytes_total",
        MetricKind::NetworkTx => "container_network_transmit_bytes_total",
        _ => return Err(ctx.unsupported()),
    };
    let series = ctx.select(metric, ctx.workload_matchers());
    Ok(ctx.sum(ctx.rate(&series)))
}

fn ingress(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    let MetricTarget::Ingress { namespace, name } = ctx.target() else {
        return Err(ctx.unsupported());
    };
    let received = ctx.request.kind == MetricKind::NetworkRx;
    if !received && ctx.request.kind != MetricKind::NetworkTx {
        return Err(ctx.unsupported());
    }
    let series = match ctx.profile.ingress {
        IngressSource::Nginx { namespace_label } => ctx.select(
            if received {
                "nginx_ingress_controller_request_size_sum"
            } else {
                "nginx_ingress_controller_response_size_sum"
            },
            vec![
                Matcher::eq("ingress", name),
                Matcher::eq(namespace_label, namespace),
            ],
        ),
        IngressSource::HaproxyRouter => {
            let quoted = regex_quote(name);
            ctx.select(
                if received {
                    "haproxy_backend_bytes_in_total"
                } else {
                    "haproxy_backend_bytes_out_total"
                },
                vec![
                    Matcher::eq("exported_namespace", namespace),
                    Matcher::re("route", &format!("{quoted}|{quoted}-[a-z0-9]+")),
                ],
            )
        }
    };
    Ok(ctx.sum(ctx.rate(&series)))
}
