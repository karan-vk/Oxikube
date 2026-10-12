//! The `@metrics/...` grammar.

use oxikube_domain::{OxiError, OxiResult};

/// What a `@metrics` mention names. Only pods so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsMention {
    /// The pod's namespace.
    pub namespace: String,
    /// The pod's name.
    pub pod: String,
}

impl MetricsMention {
    /// Reads the segments after `@metrics`: `pod/<namespace>/<name>`, or `pod/<name>` in a
    /// thread that has a default namespace.
    ///
    /// # Errors
    ///
    /// A validation error that shows the grammar, for any other shape or kind.
    pub fn parse(path: &[String], default_namespace: Option<&str>) -> OxiResult<Self> {
        let grammar = "expected @metrics/pod/<namespace>/<name>";
        let (kind, rest) = path
            .split_first()
            .ok_or_else(|| OxiError::validation(format!("@metrics names nothing: {grammar}")))?;
        if kind != "pod" {
            return Err(OxiError::validation(format!(
                "@metrics/{kind}: only pods have metrics mentions so far; {grammar}"
            )));
        }
        match rest {
            [namespace, pod] => Ok(Self {
                namespace: namespace.clone(),
                pod: pod.clone(),
            }),
            [pod] => {
                let namespace = default_namespace.ok_or_else(|| {
                    OxiError::validation(format!(
                        "@metrics/pod/{pod} has no namespace and the thread has none: {grammar}"
                    ))
                })?;
                Ok(Self {
                    namespace: namespace.to_owned(),
                    pod: pod.clone(),
                })
            }
            _ => Err(OxiError::validation(format!(
                "@metrics/pod needs a pod: {grammar}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_pod_is_named_by_namespace_and_name() {
        let m = MetricsMention::parse(&path(&["pod", "shop", "web-0"]), None).unwrap();
        assert_eq!((m.namespace.as_str(), m.pod.as_str()), ("shop", "web-0"));
    }

    #[test]
    fn the_threads_namespace_fills_a_short_mention() {
        let m = MetricsMention::parse(&path(&["pod", "web-0"]), Some("shop")).unwrap();
        assert_eq!(m.namespace, "shop");
        assert!(MetricsMention::parse(&path(&["pod", "web-0"]), None).is_err());
    }

    #[test]
    fn anything_else_shows_the_grammar() {
        for bad in [
            path(&[]),
            path(&["node", "n1"]),
            path(&["pod"]),
            path(&["pod", "a", "b", "c"]),
        ] {
            let error = MetricsMention::parse(&bad, Some("shop")).unwrap_err();
            assert!(
                error.message().contains("@metrics/pod/<namespace>/<name>"),
                "{}",
                error.message()
            );
        }
    }
}
