//! The query catalogue: every provider x scope x metric against a checked-in golden, every
//! query parsed as PromQL, and hostile names escaped.
//!
//! Goldens are `tests/golden/queries/<provider>.txt`. A change to a query shows up as a diff
//! there; regenerate with `OXIKUBE_UPDATE_GOLDENS=1 cargo test -p oxikube_prometheus --test
//! catalogue` and review the diff.

#[path = "support_catalogue/ast.rs"]
mod ast;
#[path = "support_catalogue/cases.rs"]
mod cases;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use oxikube_domain::metric_query::INSTANT_RATE_WINDOW;
use oxikube_domain::{
    LabelMatch, MetricKind, MetricQueryError, MetricRequest, MetricTarget, MetricUnit,
    MetricsProviderKind, PodSelector,
};
use oxikube_ports::QueryCataloguePort;
use oxikube_prometheus::PromqlCatalogue;

const WINDOW: Duration = Duration::from_secs(120);

fn build(
    provider: MetricsProviderKind,
    request: &MetricRequest,
) -> Result<oxikube_domain::MetricQuery, MetricQueryError> {
    PromqlCatalogue.build(provider, request, WINDOW)
}

fn golden_text(provider: MetricsProviderKind) -> String {
    let mut out = format!(
        "# {} ({}): rate window {}\n# one line per target and metric; `-` is a combination with no query\n",
        provider.label(),
        provider.id(),
        oxikube_domain::metric_query::promql_duration(WINDOW),
    );
    for (label, target) in cases::targets() {
        let _ = writeln!(out, "\n## {label}");
        for kind in MetricKind::ALL {
            let request = MetricRequest::new(target.clone(), kind);
            match build(provider, &request) {
                Ok(query) => {
                    let _ = writeln!(out, "{kind}: {}", query.promql);
                }
                Err(MetricQueryError::Unsupported { .. }) => {
                    let _ = writeln!(out, "{kind}: -");
                }
                Err(other) => panic!("{provider} {label} {kind}: {other}"),
            }
        }
    }
    out
}

#[test]
fn every_provider_scope_and_metric_matches_its_golden() {
    let update = std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/queries");
    let mut failures = Vec::new();
    for provider in cases::providers() {
        let actual = golden_text(provider);
        let path = dir.join(format!("{}.txt", provider.id()));
        if update {
            std::fs::write(&path, &actual).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (OXIKUBE_UPDATE_GOLDENS=1 writes it)",
                path.display()
            )
        });
        if expected != actual {
            let first = expected
                .lines()
                .zip(actual.lines())
                .find(|(e, a)| e != a)
                .map(|(e, a)| format!("expected: {e}\n  actual: {a}"))
                .unwrap_or_else(|| "line counts differ".into());
            failures.push(format!("{}: {first}", provider.id()));
        }
    }
    assert!(
        failures.is_empty(),
        "goldens differ (OXIKUBE_UPDATE_GOLDENS=1 to regenerate):\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_supported_combination_has_a_query_and_every_other_is_refused() {
    let mut built = 0;
    for (provider, label, request) in cases::all() {
        let supported = request.scope().supports(request.kind);
        let result = build(provider, &request);
        assert_eq!(
            result.is_ok(),
            supported,
            "{provider} {label} {}: {result:?}",
            request.kind
        );
        match result {
            Ok(query) => {
                built += 1;
                assert_eq!(query.unit, request.kind.unit());
                assert_eq!(query.series_label, request.target.series_label());
            }
            Err(error) => assert_eq!(
                error,
                MetricQueryError::Unsupported {
                    scope: request.scope(),
                    kind: request.kind
                }
            ),
        }
    }
    // 6 providers x (3 aggregate scopes x 11 + pod 9 x 2 targets + container 7 x 2 + pvc 2 + ingress 2).
    assert_eq!(built, 6 * (33 + 18 + 14 + 2 + 2));
}

#[test]
fn every_query_is_valid_promql() {
    for (provider, label, request) in cases::all() {
        if let Ok(query) = build(provider, &request) {
            ast::parse(&query.promql).unwrap_or_else(|e| {
                panic!("{provider} {label} {}: {e}\n{}", request.kind, query.promql)
            });
        }
    }
}

#[test]
fn every_query_names_its_target_in_a_selector() {
    // "Query by selector": no scope below the cluster is ever unfiltered.
    for (provider, label, request) in cases::all() {
        let Ok(query) = build(provider, &request) else {
            continue;
        };
        let matchers = ast::matchers(&query.promql);
        // A name is in the selector as a value, or one of the alternatives of a regex.
        let named = |label: &str, value: &str| {
            matchers
                .iter()
                .any(|m| m.label == label && m.value.split('|').any(|alt| alt == value))
        };
        let who = format!("{provider} {label} {}: {}", request.kind, query.promql);
        match &request.target {
            MetricTarget::Cluster => {}
            MetricTarget::Node(node) => {
                assert!(named("node", node) || named("nodename", node), "{who}");
            }
            MetricTarget::Namespace(ns) => assert!(
                named("namespace", ns) || named("exported_namespace", ns),
                "{who}"
            ),
            MetricTarget::Pod { namespace, pods } => {
                assert!(named("namespace", namespace), "{who}");
                for pod in pods.names() {
                    assert!(named("pod", pod), "{who}");
                }
            }
            MetricTarget::Container { namespace, pod, .. } => {
                assert!(named("namespace", namespace) && named("pod", pod), "{who}");
            }
            MetricTarget::Pvc { namespace, name } => {
                assert!(named("namespace", namespace), "{who}");
                assert!(named("persistentvolumeclaim", name), "{who}");
            }
            MetricTarget::Ingress { namespace, name } => {
                assert!(
                    named("namespace", namespace) || named("exported_namespace", namespace),
                    "{who}"
                );
                assert!(named("ingress", name) || named("route", name), "{who}");
            }
        }
    }
}

#[test]
fn series_labels_and_units_follow_the_request() {
    let pod = MetricTarget::Pod {
        namespace: "shop".into(),
        pods: PodSelector::One("web-0".into()),
    };
    let query = build(
        MetricsProviderKind::KubePrometheus,
        &MetricRequest::new(pod, MetricKind::CpuUsage),
    )
    .unwrap();
    assert_eq!(query.series_label, Some("pod"));
    assert_eq!(query.unit, MetricUnit::Cores);
    assert!(
        query.promql.starts_with("sum by (pod) ("),
        "{}",
        query.promql
    );

    let containers = MetricTarget::Container {
        namespace: "shop".into(),
        pod: "web-0".into(),
        container: None,
    };
    let query = build(
        MetricsProviderKind::Lens,
        &MetricRequest::new(containers, MetricKind::MemoryLimits),
    )
    .unwrap();
    assert_eq!(query.series_label, Some("container"));
    assert_eq!(query.unit, MetricUnit::Bytes);
    assert!(query.promql.starts_with("sum by (container) ("));

    let cluster = build(
        MetricsProviderKind::Mimir,
        &MetricRequest::new(MetricTarget::Cluster, MetricKind::NetworkRx),
    )
    .unwrap();
    assert_eq!(cluster.series_label, None);
    assert_eq!(cluster.unit, MetricUnit::BytesPerSecond);
}

#[test]
fn the_rate_window_reaches_every_rate() {
    for (provider, label, request) in cases::all() {
        let Ok(query) = PromqlCatalogue.build(provider, &request, Duration::from_secs(600)) else {
            continue;
        };
        let rates = query.promql.matches("rate(").count();
        let windows = query.promql.matches("[10m])").count();
        assert_eq!(
            rates, windows,
            "{provider} {label} {}: {}",
            request.kind, query.promql
        );
        assert!(!query.promql.contains("[2m]"));
    }
}

#[test]
fn instant_queries_use_the_instant_window() {
    let request = MetricRequest::new(MetricTarget::Namespace("shop".into()), MetricKind::CpuUsage);
    let instant = PromqlCatalogue
        .build_instant(MetricsProviderKind::Prometheus, &request)
        .unwrap();
    let explicit = PromqlCatalogue
        .build(
            MetricsProviderKind::Prometheus,
            &request,
            INSTANT_RATE_WINDOW,
        )
        .unwrap();
    assert_eq!(instant, explicit);
    assert!(instant.promql.contains("[5m]"));
}

#[test]
fn a_zero_window_is_refused() {
    let request = MetricRequest::new(MetricTarget::Cluster, MetricKind::CpuUsage);
    assert_eq!(
        PromqlCatalogue.build(MetricsProviderKind::Lens, &request, Duration::ZERO),
        Err(MetricQueryError::EmptyWindow)
    );
}

// --- escaping ---------------------------------------------------------------------------

/// Names that try to leave their string, add a matcher or widen a regex.
const HOSTILE: [&str; 8] = [
    r#"web"} or vector(1) # "#,
    r"back\slash\",
    "new\nline",
    r#"quote"inside"#,
    ".*",
    "(?i)web|admin",
    "a{1,3}[b]^$+?",
    "tab\there\u{7f}\u{85}",
];

fn hostile_requests(name: &str, kind: MetricKind) -> Vec<MetricRequest> {
    let ns = || name.to_owned();
    [
        MetricTarget::Node(name.into()),
        MetricTarget::Namespace(ns()),
        MetricTarget::Pod {
            namespace: ns(),
            pods: PodSelector::One(name.into()),
        },
        MetricTarget::Pod {
            namespace: "ok".into(),
            pods: PodSelector::Any(vec![name.into(), "second".into()]),
        },
        MetricTarget::Container {
            namespace: ns(),
            pod: name.into(),
            container: Some(name.into()),
        },
        MetricTarget::Pvc {
            namespace: ns(),
            name: name.into(),
        },
        MetricTarget::Ingress {
            namespace: ns(),
            name: name.into(),
        },
    ]
    .into_iter()
    .map(|target| MetricRequest::new(target, kind))
    .collect()
}

#[test]
fn hostile_names_stay_inside_their_label_value() {
    for provider in cases::providers() {
        for name in HOSTILE {
            for kind in MetricKind::ALL {
                for request in hostile_requests(name, kind) {
                    let Ok(query) = build(provider, &request) else {
                        continue;
                    };
                    let who = format!("{provider} {:?} {kind}: {}", request.target, query.promql);
                    // It parses, so the name did not break out of its string.
                    let hostile = ast::matchers(&query.promql);
                    // The same request with a harmless name has the same selectors: the name
                    // added no matcher, no metric and no label.
                    let benign_request = benign_twin(&request);
                    let benign = ast::matchers(&build(provider, &benign_request).unwrap().promql);
                    assert_eq!(shape(&hostile), shape(&benign), "{who}");
                    // And the name arrives intact as a value (or, in a regex, as an escaped one).
                    assert!(
                        hostile.iter().any(|m| m.value == name
                            || m.value.contains(&escaped(name))
                            || m.value.contains(&route_escaped(name))),
                        "{who}"
                    );
                }
            }
        }
    }
}

/// The matchers' labels, operators and metric names, without values.
fn shape(matchers: &[ast::Parsed]) -> BTreeSet<(String, String, &'static str)> {
    matchers
        .iter()
        .map(|m| (m.metric.clone(), m.label.clone(), m.op))
        .collect()
}

fn benign_twin(request: &MetricRequest) -> MetricRequest {
    let swap = |s: &str| {
        if HOSTILE.contains(&s) {
            "benign".to_owned()
        } else {
            s.to_owned()
        }
    };
    let target = match &request.target {
        MetricTarget::Node(n) => MetricTarget::Node(swap(n)),
        MetricTarget::Namespace(n) => MetricTarget::Namespace(swap(n)),
        MetricTarget::Pod { namespace, pods } => MetricTarget::Pod {
            namespace: swap(namespace),
            pods: match pods {
                PodSelector::One(p) => PodSelector::One(swap(p)),
                PodSelector::Any(p) => PodSelector::Any(p.iter().map(|p| swap(p)).collect()),
            },
        },
        MetricTarget::Container {
            namespace,
            pod,
            container,
        } => MetricTarget::Container {
            namespace: swap(namespace),
            pod: swap(pod),
            container: container.as_deref().map(swap),
        },
        MetricTarget::Pvc { namespace, name } => MetricTarget::Pvc {
            namespace: swap(namespace),
            name: swap(name),
        },
        MetricTarget::Ingress { namespace, name } => MetricTarget::Ingress {
            namespace: swap(namespace),
            name: swap(name),
        },
        MetricTarget::Cluster => MetricTarget::Cluster,
    };
    MetricRequest {
        target,
        kind: request.kind,
        extra: request.extra.clone(),
    }
}

/// RE2 `QuoteMeta` of a name, which is how it reads back out of a regex matcher.
fn escaped(name: &str) -> String {
    name.chars()
        .flat_map(|c| {
            if "\\.+*?()|[]{}^$".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

fn route_escaped(name: &str) -> String {
    escaped(name)
}

#[test]
fn a_hostile_name_cannot_inject_a_second_query() {
    let request = MetricRequest::new(
        MetricTarget::Pod {
            namespace: "shop".into(),
            pods: PodSelector::One(r#"x"} or on() vector(1) #"#.into()),
        },
        MetricKind::CpuUsage,
    );
    let query = build(MetricsProviderKind::KubePrometheus, &request).unwrap();
    assert_eq!(
        query.promql,
        r#"sum by (pod) (rate(container_cpu_usage_seconds_total{namespace="shop",pod="x\"} or on() vector(1) #",container!="",container!="POD"}[2m]))"#
    );
    let names: BTreeSet<_> = ast::matchers(&query.promql)
        .into_iter()
        .map(|m| m.metric)
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["container_cpu_usage_seconds_total".to_owned()])
    );
}

#[test]
fn a_set_of_pods_is_an_escaped_anchored_alternation() {
    let request = MetricRequest::new(
        MetricTarget::Pod {
            namespace: "shop".into(),
            pods: PodSelector::Any(vec!["web.1".into(), "web+2".into()]),
        },
        MetricKind::MemoryUsage,
    );
    let query = build(MetricsProviderKind::Prometheus, &request).unwrap();
    assert!(
        query.promql.contains(r#"pod=~"web\\.1|web\\+2""#),
        "{}",
        query.promql
    );
}

// --- extra matchers (a shared store) ---------------------------------------------------

#[test]
fn an_extra_matcher_reaches_every_selector() {
    let cluster = LabelMatch::new("cluster", r#"prod "eu""#).unwrap();
    for provider in [
        MetricsProviderKind::Mimir,
        MetricsProviderKind::VictoriaMetrics,
    ] {
        assert!(PromqlCatalogue::is_shared_store(provider));
        for (_, _, request) in cases::all().into_iter().filter(|(p, ..)| *p == provider) {
            let scoped = request.clone().with_label(cluster.clone());
            let Ok(query) = build(provider, &scoped) else {
                continue;
            };
            let matchers = ast::matchers(&query.promql);
            let selectors: BTreeSet<_> = matchers.iter().map(|m| m.metric.clone()).collect();
            for metric in selectors {
                assert!(
                    matchers.iter().any(|m| m.metric == metric
                        && m.label == "cluster"
                        && m.op == "="
                        && m.value == r#"prod "eu""#),
                    "{metric} lacks the cluster matcher: {}",
                    query.promql
                );
            }
        }
    }
    for provider in [
        MetricsProviderKind::KubePrometheus,
        MetricsProviderKind::Lens,
        MetricsProviderKind::OpenShift,
        MetricsProviderKind::Prometheus,
    ] {
        assert!(!PromqlCatalogue::is_shared_store(provider));
    }
}

// --- provider variants ------------------------------------------------------------------

fn ingress_query(provider: MetricsProviderKind, kind: MetricKind) -> String {
    let request = MetricRequest::new(
        MetricTarget::Ingress {
            namespace: "shop".into(),
            name: "web".into(),
        },
        kind,
    );
    build(provider, &request).unwrap().promql
}

#[test]
fn ingress_labels_and_sources_differ_by_stack() {
    let kube = ingress_query(MetricsProviderKind::KubePrometheus, MetricKind::NetworkRx);
    assert_eq!(
        kube,
        r#"sum(rate(nginx_ingress_controller_request_size_sum{ingress="web",exported_namespace="shop"}[2m]))"#
    );
    let lens = ingress_query(MetricsProviderKind::Lens, MetricKind::NetworkTx);
    assert_eq!(
        lens,
        r#"sum(rate(nginx_ingress_controller_response_size_sum{ingress="web",namespace="shop"}[2m]))"#
    );
    let openshift = ingress_query(MetricsProviderKind::OpenShift, MetricKind::NetworkRx);
    assert_eq!(
        openshift,
        r#"sum(rate(haproxy_backend_bytes_in_total{exported_namespace="shop",route=~"web|web-[a-z0-9]+"}[2m]))"#
    );
}

#[test]
fn nodes_are_matched_through_node_uname_info() {
    let request = MetricRequest::new(MetricTarget::Node("worker-1".into()), MetricKind::CpuUsage);
    let query = build(MetricsProviderKind::KubePrometheus, &request).unwrap();
    assert_eq!(
        query.promql,
        r#"sum(rate(node_cpu_seconds_total{mode!="idle"}[2m]) and on (instance) node_uname_info{nodename="worker-1"})"#
    );
}

#[test]
fn requests_of_aggregate_scopes_skip_finished_pods_and_named_pods_do_not() {
    let namespace = MetricRequest::new(
        MetricTarget::Namespace("shop".into()),
        MetricKind::CpuRequests,
    );
    let query = build(MetricsProviderKind::KubePrometheus, &namespace).unwrap();
    assert_eq!(
        query.promql,
        r#"sum(kube_pod_container_resource_requests{resource="cpu",namespace="shop"} * on (namespace, pod) group_left() (kube_pod_status_phase{phase=~"Pending|Running",namespace="shop"} == 1))"#
    );
    let pod = MetricRequest::new(
        MetricTarget::Container {
            namespace: "shop".into(),
            pod: "web-0".into(),
            container: None,
        },
        MetricKind::MemoryLimits,
    );
    let query = build(MetricsProviderKind::KubePrometheus, &pod).unwrap();
    assert_eq!(
        query.promql,
        r#"sum by (container) (kube_pod_container_resource_limits{resource="memory",namespace="shop",pod="web-0"})"#
    );
}

#[test]
fn unsupported_requests_say_what_is_missing() {
    let pvc = MetricRequest::new(
        MetricTarget::Pvc {
            namespace: "shop".into(),
            name: "data".into(),
        },
        MetricKind::CpuUsage,
    );
    let error = build(MetricsProviderKind::Lens, &pvc).unwrap_err();
    assert_eq!(error.to_string(), "no cpu_usage query exists at pvc scope");
    let empty = MetricRequest::new(MetricTarget::Namespace(String::new()), MetricKind::CpuUsage);
    assert_eq!(
        build(MetricsProviderKind::Lens, &empty),
        Err(MetricQueryError::EmptyName("namespace"))
    );
}
