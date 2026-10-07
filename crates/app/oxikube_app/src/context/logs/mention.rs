//! The `@logs` mention grammar.
//!
//! A mention has no whitespace, so its options are path segments:
//!
//! ```text
//! @logs/<ns>/<pod>                          a pod's default container
//! @logs/<ns>/<pod>/<container>              one container
//! @logs/<pod>                               a pod in the scope's default namespace
//! @logs/<kind>/<ns>/<name>[/<container>]    pod, deployment, statefulset, daemonset, replicaset,
//!                                           job or service (short names po, deploy, sts, ds, rs,
//!                                           svc): a workload's pods merged
//! @logs/selector/<ns>/<app=web,tier=api>    the pods a label selector picks
//! ... /--since/10m  /--since=10m  /--tail=100  /--grep=error|warn  /--container=app
//! ```
//!
//! A first segment that names a kind only counts as one with three or more segments before the
//! options (`@logs/job/default/migrate`); with two, the first is the namespace.

use std::time::Duration;

use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiError, OxiResult};

use crate::logs::{
    AggregateSource, AggregateSpec, ExcerptSource, MAX_TAIL, parse_since, workload_kind,
};

/// A parsed `@logs` mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMention {
    /// What to read.
    pub source: ExcerptSource,
    /// `--since`: only lines newer than this long ago.
    pub since: Option<Duration>,
    /// `--tail`: the most lines returned.
    pub tail: Option<usize>,
    /// `--grep`: only the lines matching this regular expression.
    pub grep: Option<String>,
}

impl LogMention {
    /// Parses the path of an `@logs` mention. `default_namespace` stands for a missing namespace.
    ///
    /// # Errors
    ///
    /// A validation error that says what the mention should look like.
    pub fn parse(path: &[String], default_namespace: Option<&str>) -> OxiResult<Self> {
        let split = path
            .iter()
            .position(|segment| segment.starts_with('-'))
            .unwrap_or(path.len());
        let (positional, options) = path.split_at(split);
        let mut container = None;
        let mut since = None;
        let mut tail = None;
        let mut grep = None;
        for (name, value) in options_of(options)? {
            match name {
                "since" => since = Some(parse_since(&value)?),
                "tail" => {
                    let n: usize = value.parse().map_err(|_| {
                        OxiError::validation(format!(
                            "--tail {value:?}: expected a number of lines"
                        ))
                    })?;
                    if n == 0 || n > MAX_TAIL {
                        return Err(OxiError::validation(format!(
                            "--tail {n}: expected 1 to {MAX_TAIL} lines"
                        )));
                    }
                    tail = Some(n);
                }
                "grep" => grep = Some(value),
                "container" | "c" => container = Some(value),
                other => {
                    return Err(OxiError::validation(format!(
                        "unknown option --{other}: use --since, --tail, --grep or --container"
                    )));
                }
            }
        }
        let source = source_of(positional, container, default_namespace)?;
        Ok(Self {
            source,
            since,
            tail,
            grep,
        })
    }
}

/// `--name=value` and `--name value...` (the value runs over the following segments, which a
/// `/` in a pattern split) as `(name, value)` pairs.
fn options_of(segments: &[String]) -> OxiResult<Vec<(&str, String)>> {
    let mut out: Vec<(&str, String)> = Vec::new();
    let mut open: Option<usize> = None;
    for segment in segments {
        if let Some(rest) = segment
            .strip_prefix("--")
            .or_else(|| segment.strip_prefix('-'))
        {
            let (name, value) = rest.split_once('=').unwrap_or((rest, ""));
            out.push((name, value.to_owned()));
            open = Some(out.len() - 1);
        } else if let Some(ix) = open {
            let value = &mut out[ix].1;
            if !value.is_empty() {
                value.push('/');
            }
            value.push_str(segment);
        } else {
            return Err(OxiError::validation(format!(
                "unexpected {segment:?} in the @logs mention"
            )));
        }
    }
    if let Some((name, _)) = out.iter().find(|(_, value)| value.is_empty()) {
        return Err(OxiError::validation(format!("--{name} needs a value")));
    }
    Ok(out)
}

fn source_of(
    positional: &[String],
    container: Option<String>,
    default_namespace: Option<&str>,
) -> OxiResult<ExcerptSource> {
    let usage = || {
        OxiError::validation(
            "expected @logs/<namespace>/<pod>[/<container>] or @logs/<kind>/<namespace>/<name>, \
             optionally with --since, --tail, --grep or --container",
        )
    };
    let kind = positional.first().and_then(|word| kind_of(word));
    match (kind, positional) {
        (Some(kind), [_, namespace, name, rest @ ..]) if rest.len() <= 1 => {
            let container = rest.first().cloned().or(container);
            Ok(match kind {
                Kind::Pod => ExcerptSource::Pod {
                    namespace: namespace.clone(),
                    pod: name.clone(),
                    container,
                },
                Kind::Selector => ExcerptSource::Workload(AggregateSpec {
                    namespace: namespace.clone(),
                    source: AggregateSource::Selector(name.clone()),
                    extra_selector: None,
                    container,
                }),
                Kind::Workload(gvk) => ExcerptSource::Workload(AggregateSpec {
                    namespace: namespace.clone(),
                    source: AggregateSource::Object {
                        gvk,
                        name: name.clone(),
                    },
                    extra_selector: None,
                    container,
                }),
            })
        }
        (_, [pod]) => Ok(ExcerptSource::Pod {
            namespace: default_namespace
                .ok_or_else(|| {
                    OxiError::validation(
                        "no namespace: write @logs/<namespace>/<pod> (the scope has no default)",
                    )
                })?
                .to_owned(),
            pod: pod.clone(),
            container,
        }),
        (_, [namespace, pod]) => Ok(ExcerptSource::Pod {
            namespace: namespace.clone(),
            pod: pod.clone(),
            container,
        }),
        (None, [namespace, pod, named]) => Ok(ExcerptSource::Pod {
            namespace: namespace.clone(),
            pod: pod.clone(),
            container: Some(named.clone()),
        }),
        _ => Err(usage()),
    }
}

enum Kind {
    Pod,
    Selector,
    Workload(Gvk),
}

fn kind_of(word: &str) -> Option<Kind> {
    match word {
        "pod" | "po" => Some(Kind::Pod),
        "selector" => Some(Kind::Selector),
        other => workload_kind(other).map(Kind::Workload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(path: &str, ns: Option<&str>) -> OxiResult<LogMention> {
        let segments: Vec<String> = path.split('/').map(str::to_owned).collect();
        LogMention::parse(&segments, ns)
    }

    fn pod(namespace: &str, pod: &str, container: Option<&str>) -> ExcerptSource {
        ExcerptSource::Pod {
            namespace: namespace.into(),
            pod: pod.into(),
            container: container.map(str::to_owned),
        }
    }

    #[test]
    fn a_pod_by_namespace_and_name_with_or_without_a_container() {
        assert_eq!(
            parse("prod/web-0", None).unwrap().source,
            pod("prod", "web-0", None)
        );
        assert_eq!(
            parse("prod/web-0/sidecar", None).unwrap().source,
            pod("prod", "web-0", Some("sidecar"))
        );
        assert_eq!(
            parse("web-0", Some("staging")).unwrap().source,
            pod("staging", "web-0", None)
        );
        assert!(
            parse("web-0", None).is_err(),
            "no namespace to fall back to"
        );
    }

    #[test]
    fn a_kind_makes_it_a_workload_read() {
        let mention = parse("deployment/prod/api", None).unwrap();
        let ExcerptSource::Workload(spec) = mention.source else {
            panic!("a workload")
        };
        assert_eq!(spec.namespace, "prod");
        assert_eq!(spec.label(), "deployment/api");
        let ExcerptSource::Workload(spec) = parse("sts/prod/db/pg", None).unwrap().source else {
            panic!()
        };
        assert_eq!(
            (spec.label().as_str(), spec.container.as_deref()),
            ("statefulset/db", Some("pg"))
        );
        let ExcerptSource::Workload(spec) = parse("selector/prod/app=web,tier=api", None)
            .unwrap()
            .source
        else {
            panic!()
        };
        assert_eq!(spec.label(), "selector app=web,tier=api");
        assert_eq!(
            parse("pod/prod/web-0", None).unwrap().source,
            pod("prod", "web-0", None)
        );
    }

    #[test]
    fn two_segments_are_always_namespace_and_pod() {
        assert_eq!(parse("job/x", None).unwrap().source, pod("job", "x", None));
    }

    #[test]
    fn options_parse_in_both_spellings() {
        let m = parse("prod/web-0/--since=10m/--tail/50/--grep=error|warn", None).unwrap();
        assert_eq!(m.since, Some(Duration::from_secs(600)));
        assert_eq!(m.tail, Some(50));
        assert_eq!(m.grep.as_deref(), Some("error|warn"));
        let m = parse("prod/web-0/--since/1h/--container=app", None).unwrap();
        assert_eq!(m.since, Some(Duration::from_secs(3_600)));
        assert_eq!(m.source, pod("prod", "web-0", Some("app")));
    }

    #[test]
    fn a_slash_in_a_pattern_is_rejoined() {
        let m = parse("prod/web-0/--grep/GET/api", None).unwrap();
        assert_eq!(m.grep.as_deref(), Some("GET/api"));
    }

    #[test]
    fn an_empty_mention_says_what_is_expected() {
        let err = LogMention::parse(&[], Some("prod")).unwrap_err();
        assert!(err.message().contains("expected @logs/"));
    }

    #[test]
    fn bad_mentions_say_what_is_expected() {
        for (path, needle) in [
            ("a/b/c/d/e", "expected @logs/"),
            ("prod/web-0/--tail=0", "1 to"),
            ("prod/web-0/--tail=lots", "number of lines"),
            ("prod/web-0/--since=soon", "duration"),
            ("prod/web-0/--follow", "needs a value"),
            ("prod/web-0/--color=red", "unknown option"),
        ] {
            let err = parse(path, None).unwrap_err();
            assert!(err.message().contains(needle), "{path}: {}", err.message());
        }
    }
}
