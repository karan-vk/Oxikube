//! Pod counts from kube-state-metrics: the pods that are pending or running.

use oxikube_domain::{MetricQueryError, MetricTarget};

use super::ctx::Ctx;
use super::selector::Matcher;

pub(super) fn build(ctx: &Ctx<'_>) -> Result<String, MetricQueryError> {
    let live = |extra: Vec<Matcher>| {
        let mut matchers = vec![Matcher::re("phase", "Pending|Running")];
        matchers.extend(extra);
        ctx.select("kube_pod_status_phase", matchers)
    };
    match ctx.target() {
        MetricTarget::Cluster => Ok(format!("sum({})", live(vec![]))),
        MetricTarget::Namespace(namespace) => Ok(format!(
            "sum({})",
            live(vec![Matcher::eq("namespace", namespace)])
        )),
        // The phase series have no node label: count the node's pods that are live.
        MetricTarget::Node(node) => {
            let info = ctx.select("kube_pod_info", vec![Matcher::eq("node", node)]);
            let live = live(vec![]);
            Ok(format!(
                "count({info} and on (namespace, pod) (sum by (namespace, pod) ({live}) > 0))"
            ))
        }
        _ => Err(ctx.unsupported()),
    }
}
