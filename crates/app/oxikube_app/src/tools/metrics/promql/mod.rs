//! `k8s.query_promql`: a PromQL query against the cluster's Prometheus, raw or from the query
//! catalogue.

mod args;
mod render;

use async_trait::async_trait;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metric_query::{INSTANT_RATE_WINDOW, MetricKind, MetricScope};
use oxikube_domain::redact::redact;
use oxikube_domain::{ErrorKind, MetricsProviderKind, OxiError, OxiResult};
use oxikube_ports::{
    PromqlError, PromqlSeries, RangePlan, TimeRange, ToolAnnotations, ToolContext, ToolDef,
    ToolName, ToolOutput, ToolPort,
};
use serde_json::{Value, json};

use self::args::{Args, Form, validation};
use self::render::{Source, render};
use super::bounds::{MAX_QUERY_CHARS, QUERY_DEADLINE};
use super::cluster::resolve;
use super::deadline::within;
use super::install::MetricsAgentDeps;
use super::unavailable::unavailable;

/// The tool's name.
pub const QUERY_PROMQL: &str = "k8s.query_promql";

/// The `k8s.query_promql` tool: see the [module docs](super).
pub struct QueryPromqlTool {
    def: ToolDef,
    deps: MetricsAgentDeps,
}

impl QueryPromqlTool {
    /// The tool over `deps`.
    pub fn new(deps: MetricsAgentDeps) -> Self {
        Self {
            def: definition(),
            deps,
        }
    }

    /// Runs `promql` as an instant or a range query on `cluster`. `Err` is the port's error.
    async fn run(
        &self,
        cluster: &ClusterId,
        promql: &str,
        range: Option<TimeRange>,
    ) -> Option<OxiResult<Vec<PromqlSeries>>> {
        let port = &self.deps.promql;
        let call = async {
            match range {
                Some(range) => port.query_range(cluster, promql, range).await,
                None => port.query_instant(cluster, promql, None).await,
            }
        };
        within(self.deps.clock.as_ref(), QUERY_DEADLINE, call).await
    }
}

impl std::fmt::Debug for QueryPromqlTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryPromqlTool").finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for QueryPromqlTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let args = Args::parse(&args)?;
        let session = match resolve(&self.deps.sessions, ctx)? {
            Ok(session) => session,
            Err(output) => return Ok(output),
        };
        let (cluster, title) = (session.id().clone(), session.title().to_owned());

        // Metrics turned off for this cluster, no stack found, or one that is still being looked
        // for: all visible, none of them a query.
        let status = self.deps.provider.status(&cluster);
        if !self.deps.provider.metrics_enabled(&cluster) {
            return Ok(unavailable(&title, "turned off in this cluster's settings"));
        }
        if let Some(reason) = status.unavailable_reason() {
            return Ok(unavailable(&title, &reason));
        }
        let Some(provider) = status.provider() else {
            return Ok(unavailable(
                &title,
                "still looking for a Prometheus-compatible service; try again in a moment",
            ));
        };
        let kind: MetricsProviderKind = provider.kind;

        let now = self.deps.clock.now();
        let plan = args.range.map(|span| RangePlan::ending_at(now, span));
        let (promql, unit, note) = match &args.form {
            Form::Raw(text) => (text.clone(), None, None),
            Form::Catalogue(request) => {
                let window = plan.map_or(INSTANT_RATE_WINDOW, |p| p.rate_window);
                let query = self
                    .deps
                    .catalogue
                    .build(kind, request, window)
                    .map_err(validation)?;
                // Mimir and VictoriaMetrics usually hold many clusters; without a matcher the
                // answer may mix them, and the model should know.
                let note = (request.extra.is_empty()
                    && matches!(
                        kind,
                        MetricsProviderKind::Mimir | MetricsProviderKind::VictoriaMetrics
                    ))
                .then_some(
                    "this store may hold several clusters: pass `labels` (for example \
                     {\"cluster\": \"<name>\"}) to narrow the query to one",
                );
                (query.promql, Some(query.unit), note)
            }
        };
        let time_range = plan.map(|p| p.range);

        let series = match self.run(&cluster, &promql, time_range).await {
            None => {
                return Ok(unavailable(&title, "Prometheus did not answer in time"));
            }
            Some(Ok(series)) => series,
            Some(Err(error)) => return Ok(failed(&title, &error)),
        };
        let source = Source {
            cluster: &title,
            provider: kind.label(),
            promql: &promql,
            unit,
            range: args.range,
            note,
        };
        Ok(render(&source, series))
    }
}

/// A failed call: a query Prometheus rejects is a tool error (the model can fix it); anything
/// else means there is no usable Prometheus right now, which is an `unavailable` result.
fn failed(cluster: &str, error: &OxiError) -> ToolOutput {
    let rejected = PromqlError::of(error).is_some_and(PromqlError::is_query_error)
        || (error.kind() == ErrorKind::Validation && PromqlError::of(error).is_none());
    if rejected {
        return ToolOutput::error(format!(
            "Prometheus rejected the query: {}",
            redact(error.message())
        ));
    }
    unavailable(cluster, error.message())
}

fn definition() -> ToolDef {
    let name = ToolName::new(QUERY_PROMQL).expect("a well-formed tool name");
    let scopes = MetricScope::ALL.map(MetricScope::id);
    let metrics = MetricKind::ALL.map(MetricKind::id);
    ToolDef::read_only(
        name,
        "Query the cluster's Prometheus-compatible metrics. Prefer the catalogue form (`scope` + \
         `metric` + `name`): it writes the right PromQL for the stack the cluster has \
         (kube-prometheus, Lens, VictoriaMetrics, Mimir, OpenShift). Use `query` for raw PromQL. \
         Without `range` the query is instant (the value now); with `range` (for example `1h`) \
         it returns points over that span, ending now. The answer is bounded: at most 20 series \
         with their latest values and a thinned set of [unix seconds, value] points, rounded. \
         If the cluster has no Prometheus, metrics are turned off for it, or it cannot be \
         reached, the result has status \"unavailable\" and the reason.",
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_QUERY_CHARS,
                    "description": "Raw PromQL. Do not combine with scope/metric."
                },
                "scope": {
                    "type": "string",
                    "enum": scopes,
                    "description": "The level to measure (catalogue form)."
                },
                "metric": {
                    "type": "string",
                    "enum": metrics,
                    "description": "What to measure (catalogue form). Not every metric exists at every scope."
                },
                "name": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The object's name: the node, the namespace (scope namespace), the pod (pod and container), the claim or the ingress."
                },
                "namespace": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The object's namespace (pod, container, pvc and ingress scopes)."
                },
                "container": {
                    "type": "string",
                    "minLength": 1,
                    "description": "A container of the pod (scope container); default every container."
                },
                "labels": {
                    "type": "object",
                    "description": "Extra label matchers added to every selector, for a store shared by several clusters, for example {\"cluster\": \"prod\"}."
                },
                "range": {
                    "type": "string",
                    "minLength": 2,
                    "description": "A range query over this span ending now: 15m, 1h, 6h, 1d ... from 1m to 7d. Default: an instant query."
                }
            },
            "additionalProperties": false
        }),
    )
    .with_title("Query PromQL")
    .with_annotations(ToolAnnotations {
        idempotent: true,
        // A configured endpoint may be outside the cluster.
        open_world: true,
        ..ToolAnnotations::default()
    })
}
