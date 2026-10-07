//! The arguments of `k8s.get_logs`, parsed and validated into an [`ExcerptRequest`].

use oxikube_domain::{OxiError, OxiResult};
use serde::Deserialize;
use serde_json::Value;

use crate::logs::{
    AggregateSource, AggregateSpec, DEFAULT_TAIL, ExcerptRequest, ExcerptSource, LogFilter,
    MAX_TAIL, parse_since, workload_kind,
};

/// Namespace used when the call names none.
pub(super) const DEFAULT_NAMESPACE: &str = "default";

/// What the model passed, before it is checked.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetLogsArgs {
    pod: Option<String>,
    selector: Option<String>,
    namespace: Option<String>,
    container: Option<String>,
    since: Option<String>,
    tail: Option<u64>,
    grep: Option<String>,
}

impl GetLogsArgs {
    /// Reads `args` (an object matching the tool's input schema).
    pub(super) fn parse(args: Value) -> OxiResult<Self> {
        serde_json::from_value(args)
            .map_err(|error| OxiError::validation(format!("get_logs arguments: {error}")))
    }

    /// The bounded read these arguments ask for.
    ///
    /// # Errors
    ///
    /// A validation error unless exactly one of `pod` and `selector` is given, for a `tail` over
    /// [`MAX_TAIL`] (or zero), and for a `since` that is not a duration.
    pub(super) fn into_request(self) -> OxiResult<ExcerptRequest> {
        let given = |value: Option<String>| value.filter(|v| !v.trim().is_empty());
        let (pod, selector) = (given(self.pod), given(self.selector));
        let namespace = given(self.namespace).unwrap_or_else(|| DEFAULT_NAMESPACE.to_owned());
        let container = given(self.container);

        let source = match (pod, selector) {
            (Some(pod), None) => ExcerptSource::Pod {
                namespace,
                pod,
                container,
            },
            (None, Some(selector)) => ExcerptSource::Workload(AggregateSpec {
                namespace,
                source: selector_source(selector.trim()),
                extra_selector: None,
                container,
            }),
            (Some(_), Some(_)) => {
                return Err(OxiError::validation(
                    "give either pod or selector, not both",
                ));
            }
            (None, None) => {
                return Err(OxiError::validation(
                    "give pod (one pod's logs) or selector (the logs of every pod it picks)",
                ));
            }
        };

        let mut request = ExcerptRequest::new(source);
        if let Some(tail) = self.tail {
            let tail = usize::try_from(tail).unwrap_or(usize::MAX);
            if tail == 0 || tail > MAX_TAIL {
                return Err(OxiError::validation(format!(
                    "tail {tail}: expected 1 to {MAX_TAIL} lines (the default is {DEFAULT_TAIL})"
                )));
            }
            request = request.tail(tail);
        }
        if let Some(since) = given(self.since) {
            request = request.since(parse_since(&since)?);
        }
        if let Some(grep) = given(self.grep) {
            request = request.matching(LogFilter::new(grep));
        }
        Ok(request)
    }
}

/// `deployment/api` names a workload; anything else is a label selector (`app=web,tier=api`).
/// A label key may contain `/` (`app.kubernetes.io/name=web`), but then an operator follows.
fn selector_source(selector: &str) -> AggregateSource {
    if let Some((kind, name)) = selector.split_once('/')
        && !name.is_empty()
        && !selector.contains(['=', '!', ',', ' ', '(', ')'])
        && let Some(gvk) = workload_kind(kind)
    {
        return AggregateSource::Object {
            gvk,
            name: name.to_owned(),
        };
    }
    AggregateSource::Selector(selector.to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use oxikube_domain::ErrorKind;
    use serde_json::json;

    use super::*;

    fn request(args: Value) -> OxiResult<ExcerptRequest> {
        GetLogsArgs::parse(args)?.into_request()
    }

    #[test]
    fn a_pod_with_defaults() {
        let r = request(json!({"pod": "web-0"})).unwrap();
        assert_eq!(
            r.source,
            ExcerptSource::Pod {
                namespace: "default".into(),
                pod: "web-0".into(),
                container: None
            }
        );
        assert_eq!(r.tail, DEFAULT_TAIL);
        assert!(r.since.is_none() && r.filter.is_empty());
    }

    #[test]
    fn every_option_is_read() {
        let r = request(json!({
            "pod": "web-0", "namespace": "prod", "container": "app",
            "since": "15m", "tail": 50, "grep": "error|warn"
        }))
        .unwrap();
        assert_eq!(
            r.source,
            ExcerptSource::Pod {
                namespace: "prod".into(),
                pod: "web-0".into(),
                container: Some("app".into())
            }
        );
        assert_eq!((r.tail, r.since), (50, Some(Duration::from_secs(900))));
        assert_eq!(r.filter.pattern, "error|warn");
    }

    #[test]
    fn a_selector_is_a_label_selector_or_a_workload() {
        let ExcerptSource::Workload(spec) = request(json!({"selector": "app=web,tier=api"}))
            .unwrap()
            .source
        else {
            panic!()
        };
        assert_eq!(
            spec.source,
            AggregateSource::Selector("app=web,tier=api".into())
        );
        let ExcerptSource::Workload(spec) =
            request(json!({"selector": "deployment/api", "namespace": "prod"}))
                .unwrap()
                .source
        else {
            panic!()
        };
        assert_eq!(spec.label(), "deployment/api");
        // A label key with a slash is still a selector.
        let ExcerptSource::Workload(spec) =
            request(json!({"selector": "app.kubernetes.io/name=web"}))
                .unwrap()
                .source
        else {
            panic!()
        };
        assert!(matches!(spec.source, AggregateSource::Selector(_)));
    }

    #[test]
    fn exactly_one_of_pod_and_selector() {
        for args in [
            json!({}),
            json!({"pod": " ", "selector": ""}),
            json!({"pod": "a", "selector": "app=b"}),
        ] {
            let err = request(args.clone()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation, "{args}");
        }
    }

    #[test]
    fn tail_since_and_unknown_arguments_are_checked() {
        for (args, needle) in [
            (json!({"pod": "a", "tail": 0}), "expected 1 to"),
            (json!({"pod": "a", "tail": 100_000}), "expected 1 to"),
            (json!({"pod": "a", "since": "yesterday"}), "duration"),
            (json!({"pod": "a", "follow": true}), "unknown field"),
            (json!({"pod": 7}), "invalid type"),
        ] {
            let err = request(args.clone()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation, "{args}");
            assert!(err.message().contains(needle), "{args}: {}", err.message());
        }
    }
}
