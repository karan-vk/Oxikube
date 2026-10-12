//! Cluster and node scopes: whole-machine numbers from node-exporter.
//!
//! The cluster is the sum of its nodes, so a node's usage compares with its allocatable and the
//! cluster's with the sum. A node is picked through `node_uname_info{nodename="<node>"}`, which
//! every node-exporter exposes whatever its `instance` label holds (a node name, an IP and port).

use oxikube_domain::{MetricKind, MetricQueryError, MetricTarget};

use super::ctx::Ctx;
use super::selector::{Matcher, Selector};

/// Interfaces that are not the machine's own traffic: loopback, virtual ethernet pairs, bridges
/// and the overlay devices of the common CNIs.
const VIRTUAL_DEVICES: &str = "lo|veth.*|docker.*|br-.*|cni.*|flannel.*|cali.*|cilium.*|lxc.*|tunl.*|vxlan.*|kube-.*|virbr.*|weave|tap.*";

/// Filesystems that are not storage, left out of the root mount's numbers.
const NON_STORAGE_FILESYSTEMS: &str = "tmpfs|ramfs|squashfs";

pub(super) fn build(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    let inner = match ctx.request.kind {
        MetricKind::CpuUsage => {
            ctx.rate(&ctx.select("node_cpu_seconds_total", vec![Matcher::ne("mode", "idle")]))
        }
        MetricKind::MemoryUsage => difference(
            ctx,
            ctx.select("node_memory_MemTotal_bytes", vec![]),
            ctx.select("node_memory_MemAvailable_bytes", vec![]),
        ),
        MetricKind::NetworkRx => ctx.rate(&device("node_network_receive_bytes_total", ctx)),
        MetricKind::NetworkTx => ctx.rate(&device("node_network_transmit_bytes_total", ctx)),
        MetricKind::DiskUsage => difference(
            ctx,
            root_fs("node_filesystem_size_bytes", ctx),
            root_fs("node_filesystem_avail_bytes", ctx),
        ),
        MetricKind::DiskCapacity => root_fs("node_filesystem_size_bytes", ctx).to_string(),
        _ => return Err(ctx.unsupported()),
    };
    Ok(match ctx.target() {
        MetricTarget::Node(node) => {
            let uname = ctx.select("node_uname_info", vec![Matcher::eq("nodename", node)]);
            format!("sum({inner} and on (instance) {uname})")
        }
        _ => format!("sum({inner})"),
    })
}

/// `a - b`, parenthesised when it is the left side of the node filter.
fn difference(ctx: &Ctx<'_>, a: Selector, b: Selector) -> String {
    match ctx.target() {
        MetricTarget::Node(_) => format!("({a} - {b})"),
        _ => format!("{a} - {b}"),
    }
}

fn device(metric: &'static str, ctx: &Ctx<'_>) -> Selector {
    ctx.select(metric, vec![Matcher::nre("device", VIRTUAL_DEVICES)])
}

fn root_fs(metric: &'static str, ctx: &Ctx<'_>) -> Selector {
    ctx.select(
        metric,
        vec![
            Matcher::eq("mountpoint", "/"),
            Matcher::nre("fstype", NON_STORAGE_FILESYSTEMS),
        ],
    )
}
