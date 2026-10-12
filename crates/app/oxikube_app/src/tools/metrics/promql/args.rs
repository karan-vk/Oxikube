//! The arguments of `k8s.query_promql`: a raw query, or a catalogue selector.

use std::time::Duration;

use oxikube_domain::metric_query::{
    LabelMatch, MetricKind, MetricRequest, MetricScope, MetricTarget, PodSelector,
};
use oxikube_domain::{OxiError, OxiResult};
use serde_json::Value;

use crate::tools::metrics::bounds::{MAX_QUERY_CHARS, MAX_RANGE, MIN_RANGE};

/// Most extra label matchers of a catalogue query.
const MAX_LABELS: usize = 8;

/// Which of the two forms the call used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Form {
    /// PromQL text, run as given.
    Raw(String),
    /// A catalogue request, turned into PromQL for the cluster's stack.
    Catalogue(MetricRequest),
}

/// A parsed call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Args {
    pub(super) form: Form,
    /// The span of a range query; `None` for an instant one.
    pub(super) range: Option<Duration>,
}

/// The catalogue form's arguments; naming any of them with `query` is a mistake.
const SELECTOR_ARGS: [&str; 6] = [
    "scope",
    "metric",
    "name",
    "namespace",
    "container",
    "labels",
];

impl Args {
    /// Reads `args` (already checked against the tool's schema).
    ///
    /// # Errors
    ///
    /// A validation error for neither form or both, an empty or over-long query, an unknown
    /// scope or metric, a missing name, a scope and metric the catalogue has no query for, a bad
    /// label, or a `range` outside 1 minute to 7 days.
    pub(super) fn parse(args: &Value) -> OxiResult<Self> {
        let query = args.get("query").filter(|v| !v.is_null());
        let selector = SELECTOR_ARGS
            .iter()
            .any(|name| args.get(name).is_some_and(|v| !v.is_null()));
        let form = match (query, selector) {
            (Some(_), true) => {
                return Err(OxiError::validation(
                    "give either `query` (raw PromQL) or `scope` and `metric` (the catalogue), not both",
                ));
            }
            (None, false) => {
                return Err(OxiError::validation(
                    "give `query` (raw PromQL) or `scope` and `metric` (the catalogue)",
                ));
            }
            (Some(query), false) => Form::Raw(raw_query(query)?),
            (None, true) => Form::Catalogue(catalogue_request(args)?),
        };
        Ok(Self {
            form,
            range: args
                .get("range")
                .filter(|v| !v.is_null())
                .map(range)
                .transpose()?,
        })
    }
}

fn raw_query(value: &Value) -> OxiResult<String> {
    let text = value
        .as_str()
        .ok_or_else(|| OxiError::validation("`query` must be a string"))?
        .trim();
    if text.is_empty() {
        return Err(OxiError::validation("`query` is empty"));
    }
    if text.chars().count() > MAX_QUERY_CHARS {
        return Err(OxiError::validation(format!(
            "`query` is longer than {MAX_QUERY_CHARS} characters"
        )));
    }
    Ok(text.to_owned())
}

fn range(value: &Value) -> OxiResult<Duration> {
    let bad = || {
        OxiError::validation(format!(
            "range {value}: expected a duration like 15m, 6h or 1d, from {} to {}",
            short(MIN_RANGE),
            short(MAX_RANGE)
        ))
    };
    let span = crate::logs::parse_since(value.as_str().ok_or_else(bad)?).map_err(|_| bad())?;
    if span < MIN_RANGE || span > MAX_RANGE {
        return Err(bad());
    }
    Ok(span)
}

fn short(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs.is_multiple_of(86_400) {
        format!("{}d", secs / 86_400)
    } else {
        format!("{}m", secs / 60)
    }
}

/// A string argument, trimmed; `None` when absent.
fn text<'a>(args: &'a Value, name: &str) -> OxiResult<Option<&'a str>> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim())),
        Some(_) => Err(OxiError::validation(format!(
            "`{name}` must be a non-empty string"
        ))),
    }
}

fn required<'a>(args: &'a Value, name: &str, scope: MetricScope) -> OxiResult<&'a str> {
    text(args, name)?
        .ok_or_else(|| OxiError::validation(format!("`{name}` is required for scope {scope}")))
}

fn catalogue_request(args: &Value) -> OxiResult<MetricRequest> {
    let scope_id = text(args, "scope")?
        .ok_or_else(|| OxiError::validation("`scope` is required with the catalogue form"))?;
    let scope = MetricScope::ALL
        .into_iter()
        .find(|s| s.id() == scope_id)
        .ok_or_else(|| {
            OxiError::validation(format!(
                "scope {scope_id:?}: expected one of {}",
                MetricScope::ALL.map(MetricScope::id).join(", ")
            ))
        })?;
    let metric_id = text(args, "metric")?
        .ok_or_else(|| OxiError::validation("`metric` is required with the catalogue form"))?;
    let kind = MetricKind::ALL
        .into_iter()
        .find(|k| k.id() == metric_id)
        .ok_or_else(|| {
            OxiError::validation(format!(
                "metric {metric_id:?}: expected one of {}",
                MetricKind::ALL.map(MetricKind::id).join(", ")
            ))
        })?;

    let namespace = |args: &Value| required(args, "namespace", scope).map(str::to_owned);
    let name = |args: &Value| required(args, "name", scope).map(str::to_owned);
    let target = match scope {
        MetricScope::Cluster => MetricTarget::Cluster,
        MetricScope::Node => MetricTarget::Node(name(args)?),
        // A namespace is named by `name`, or by `namespace` as everywhere else.
        MetricScope::Namespace => MetricTarget::Namespace(match text(args, "name")? {
            Some(name) => name.to_owned(),
            None => namespace(args)?,
        }),
        MetricScope::Pod => MetricTarget::Pod {
            namespace: namespace(args)?,
            pods: PodSelector::One(name(args)?),
        },
        MetricScope::Container => MetricTarget::Container {
            namespace: namespace(args)?,
            pod: name(args)?,
            container: text(args, "container")?.map(str::to_owned),
        },
        MetricScope::Pvc => MetricTarget::Pvc {
            namespace: namespace(args)?,
            name: name(args)?,
        },
        MetricScope::Ingress => MetricTarget::Ingress {
            namespace: namespace(args)?,
            name: name(args)?,
        },
    };
    let mut request = MetricRequest::new(target, kind);
    request.validate().map_err(validation)?;
    for label in labels(args)? {
        request = request.with_label(label);
    }
    Ok(request)
}

fn labels(args: &Value) -> OxiResult<Vec<LabelMatch>> {
    let Some(value) = args.get("labels").filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let map = value
        .as_object()
        .ok_or_else(|| OxiError::validation("`labels` must be an object of label to value"))?;
    if map.len() > MAX_LABELS {
        return Err(OxiError::validation(format!(
            "`labels` has more than {MAX_LABELS} entries"
        )));
    }
    map.iter()
        .map(|(name, value)| {
            let value = value.as_str().ok_or_else(|| {
                OxiError::validation(format!("label {name:?}: the value must be a string"))
            })?;
            LabelMatch::new(name.as_str(), value).map_err(validation)
        })
        .collect()
}

/// A request the catalogue cannot serve, as a validation error.
pub(super) fn validation(error: oxikube_domain::metric_query::MetricQueryError) -> OxiError {
    OxiError::validation(error.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request(args: Value) -> MetricRequest {
        match Args::parse(&args).expect("parses").form {
            Form::Catalogue(request) => request,
            Form::Raw(_) => panic!("a catalogue form"),
        }
    }

    #[test]
    fn every_scope_reads_its_target() {
        assert_eq!(
            request(json!({"scope": "cluster", "metric": "pod_count"})).target,
            MetricTarget::Cluster
        );
        assert_eq!(
            request(json!({"scope": "node", "metric": "cpu_usage", "name": "n1"})).target,
            MetricTarget::Node("n1".into())
        );
        for args in [
            json!({"scope": "namespace", "metric": "cpu_usage", "name": "shop"}),
            json!({"scope": "namespace", "metric": "cpu_usage", "namespace": "shop"}),
        ] {
            assert_eq!(request(args).target, MetricTarget::Namespace("shop".into()));
        }
        assert_eq!(
            request(json!({"scope": "container", "metric": "memory_usage", "namespace": "a", "name": "p"}))
                .target,
            MetricTarget::Container { namespace: "a".into(), pod: "p".into(), container: None }
        );
        assert_eq!(
            request(json!({"scope": "container", "metric": "memory_usage", "namespace": "a", "name": "p", "container": "c"}))
                .target,
            MetricTarget::Container { namespace: "a".into(), pod: "p".into(), container: Some("c".into()) }
        );
        assert_eq!(
            request(
                json!({"scope": "pvc", "metric": "disk_usage", "namespace": "a", "name": "data"})
            )
            .target,
            MetricTarget::Pvc {
                namespace: "a".into(),
                name: "data".into()
            }
        );
        assert_eq!(
            request(
                json!({"scope": "ingress", "metric": "network_rx", "namespace": "a", "name": "web"})
            )
            .target,
            MetricTarget::Ingress {
                namespace: "a".into(),
                name: "web".into()
            }
        );
    }

    #[test]
    fn a_call_names_one_form_and_a_valid_range() {
        let raw = Args::parse(&json!({"query": " up ", "range": "90m"})).unwrap();
        assert_eq!(raw.form, Form::Raw("up".into()));
        assert_eq!(raw.range, Some(Duration::from_secs(90 * 60)));
        assert_eq!(Args::parse(&json!({"query": "up"})).unwrap().range, None);
        for bad in [
            json!({}),
            json!({"query": "up", "metric": "cpu_usage"}),
            json!({"query": 5}),
            json!({"query": "up", "range": "1s"}),
            json!({"query": "up", "range": "30d"}),
            json!({"query": "up", "range": 5}),
            json!({"scope": "cluster"}),
            json!({"metric": "cpu_usage"}),
            json!({"scope": "cluster", "metric": "cpu_usage", "labels": "x"}),
            json!({"scope": "cluster", "metric": "cpu_usage", "labels": {"a": 1}}),
        ] {
            let error = Args::parse(&bad).expect_err(&bad.to_string());
            assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation, "{bad}");
        }
    }

    #[test]
    fn extra_labels_become_matchers() {
        let r = request(
            json!({"scope": "node", "metric": "cpu_usage", "name": "n", "labels": {"cluster": "prod"}}),
        );
        assert_eq!(r.extra.len(), 1);
        assert_eq!((r.extra[0].name(), r.extra[0].value()), ("cluster", "prod"));
    }
}
