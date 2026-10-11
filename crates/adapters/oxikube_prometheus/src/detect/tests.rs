//! Detection against fake Service lists and a fake service proxy that answers like each stack.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::{MetricsProviderKind, OxiError};
use oxikube_ports::{
    DetectionFailure, DetectionResult, DetectionState, ListPage, ProbeOutcome,
    ProviderDetectorPort, ProviderTarget, ProxyResponse, ServicePort, ServiceProxyPort,
};
use oxikube_testkit::{FakeResourcePort, FakeServiceProxyPort};
use serde_json::json;

use super::{Catalogue, ProviderDetector};

fn service(namespace: &str, name: &str, ports: serde_json::Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": name, "namespace": namespace },
        "spec": { "ports": ports }
    }))
    .unwrap()
}

fn named(namespace: &str, name: &str, port_name: &str, port: u16) -> Resource {
    service(
        namespace,
        name,
        json!([{ "name": port_name, "port": port }]),
    )
}

/// A proxy that answers like a Prometheus API at `service`'s `path` only.
struct Backend {
    namespace: &'static str,
    name_prefix: &'static str,
    path: &'static str,
}

fn proxy(backends: Vec<Backend>) -> Arc<FakeServiceProxyPort> {
    let fake = Arc::new(FakeServiceProxyPort::new());
    fake.respond_with(move |service, request| {
        let hit = backends.iter().find(|b| {
            b.namespace == service.namespace && service.name.starts_with(b.name_prefix)
        })?;
        if request.path == hit.path {
            Some(Ok(FakeServiceProxyPort::prometheus_ok()))
        } else {
            // What a Prometheus says to a path it does not serve.
            Some(Ok(ProxyResponse {
                status: 404,
                content_type: Some("text/plain".into()),
                body: b"404 page not found".to_vec(),
                truncated: false,
                api_server_message: None,
            }))
        }
    });
    fake
}

async fn detect(
    detector: &ProviderDetector,
    resources: Vec<Resource>,
    proxy: Arc<FakeServiceProxyPort>,
) -> DetectionResult {
    let resources = Arc::new(FakeResourcePort::new().with_objects(resources));
    detector.detect(resources, proxy).await
}

fn found(result: &DetectionResult) -> &oxikube_ports::DetectedProvider {
    let DetectionState::Found(provider) = &result.state else {
        panic!("expected Found, got {:?}", result.state);
    };
    provider
}

fn service_of(result: &DetectionResult) -> String {
    let ProviderTarget::Service(service) = &found(result).target else {
        panic!("expected a service target");
    };
    service.to_string()
}

#[tokio::test]
async fn each_stacks_service_layout_is_found_and_classified() {
    struct Case {
        resources: Vec<Resource>,
        backend: Backend,
        kind: MetricsProviderKind,
        service: &'static str,
        prefix: Option<&'static str>,
    }
    let cases = [
        Case {
            resources: vec![named("monitoring", "prometheus-operated", "web", 9090)],
            backend: Backend {
                namespace: "monitoring",
                name_prefix: "prometheus-operated",
                path: "/api/v1/query",
            },
            kind: MetricsProviderKind::KubePrometheus,
            service: "monitoring/prometheus-operated:web",
            prefix: None,
        },
        Case {
            resources: vec![named(
                "monitoring",
                "kube-prometheus-stack-prometheus",
                "http-web",
                9090,
            )],
            backend: Backend {
                namespace: "monitoring",
                name_prefix: "kube-prometheus-stack-prometheus",
                path: "/api/v1/query",
            },
            kind: MetricsProviderKind::KubePrometheus,
            service: "monitoring/kube-prometheus-stack-prometheus:http-web",
            prefix: None,
        },
        Case {
            resources: vec![service(
                "lens-metrics",
                "prometheus",
                json!([{ "port": 80 }]),
            )],
            backend: Backend {
                namespace: "lens-metrics",
                name_prefix: "prometheus",
                path: "/api/v1/query",
            },
            kind: MetricsProviderKind::Lens,
            service: "lens-metrics/prometheus:80",
            prefix: None,
        },
        Case {
            resources: vec![named("monitoring", "vmsingle-vmks", "http", 8429)],
            backend: Backend {
                namespace: "monitoring",
                name_prefix: "vmsingle",
                path: "/api/v1/query",
            },
            kind: MetricsProviderKind::VictoriaMetrics,
            service: "monitoring/vmsingle-vmks:http",
            prefix: None,
        },
        Case {
            resources: vec![named(
                "victoria-metrics",
                "vmselect-vmcluster",
                "http",
                8481,
            )],
            backend: Backend {
                namespace: "victoria-metrics",
                name_prefix: "vmselect",
                path: "/select/0/prometheus/api/v1/query",
            },
            kind: MetricsProviderKind::VictoriaMetrics,
            service: "victoria-metrics/vmselect-vmcluster:http",
            prefix: Some("/select/0/prometheus"),
        },
        Case {
            resources: vec![named(
                "observability",
                "mimir-query-frontend",
                "http-metrics",
                8080,
            )],
            backend: Backend {
                namespace: "observability",
                name_prefix: "mimir-query-frontend",
                path: "/prometheus/api/v1/query",
            },
            kind: MetricsProviderKind::Mimir,
            service: "observability/mimir-query-frontend:http-metrics",
            prefix: Some("/prometheus"),
        },
        Case {
            resources: vec![named("openshift-monitoring", "thanos-querier", "web", 9091)],
            backend: Backend {
                namespace: "openshift-monitoring",
                name_prefix: "thanos-querier",
                path: "/api/v1/query",
            },
            kind: MetricsProviderKind::OpenShift,
            service: "openshift-monitoring/https:thanos-querier:web",
            prefix: None,
        },
    ];
    let detector = ProviderDetector::new();
    for case in cases {
        let result = detect(&detector, case.resources, proxy(vec![case.backend])).await;
        let provider = found(&result);
        assert_eq!(provider.kind, case.kind, "{}", case.service);
        assert_eq!(service_of(&result), case.service);
        assert_eq!(
            provider.path_prefix.as_deref(),
            case.prefix,
            "{}",
            case.service
        );
        assert!(result.finished_at.is_some());
    }
}

#[tokio::test]
async fn the_lens_stack_is_found_under_its_path_prefix_when_the_root_does_not_answer() {
    let backend = Backend {
        namespace: "lens-metrics",
        name_prefix: "prometheus",
        path: "/prometheus/api/v1/query",
    };
    let result = detect(
        &ProviderDetector::new(),
        vec![service(
            "lens-metrics",
            "prometheus",
            json!([{ "port": 80 }]),
        )],
        proxy(vec![backend]),
    )
    .await;
    assert_eq!(found(&result).kind, MetricsProviderKind::Lens);
    assert_eq!(found(&result).path_prefix.as_deref(), Some("/prometheus"));
}

#[tokio::test]
async fn no_candidates_is_not_found_and_every_service_looked_for_is_listed() {
    let detector = ProviderDetector::new();
    let result = detect(
        &detector,
        vec![named("default", "web", "http", 80)],
        Arc::new(FakeServiceProxyPort::new()),
    )
    .await;
    assert_eq!(result.state, DetectionState::NotFound);
    let expected: usize = detector
        .catalogue()
        .entries
        .iter()
        .map(|entry| entry.namespaces.len())
        .sum();
    assert_eq!(result.probes.len(), expected);
    assert!(
        result
            .probes
            .iter()
            .all(|p| p.outcome == ProbeOutcome::NotPresent)
    );
    let labels: Vec<String> = result.probes.iter().map(|p| p.label()).collect();
    for want in [
        "monitoring/prometheus-operated",
        "monitoring/kube-prometheus-stack-prometheus",
        "lens-metrics/prometheus",
        "monitoring/vmsingle*",
        "victoria-metrics/vmselect*",
        "observability/mimir-query-frontend",
        "openshift-monitoring/thanos-querier",
    ] {
        assert!(
            labels.iter().any(|l| l == want),
            "{want} missing from {labels:?}"
        );
    }
}

fn script_lists(
    resources: &FakeResourcePort,
    detector: &ProviderDetector,
    forbidden: &[&str],
    objects: &[Resource],
) {
    // One list per namespace, in sorted order (the order the detector starts them).
    for namespace in detector.catalogue().namespaces() {
        if forbidden.contains(&namespace) {
            resources
                .script()
                .list
                .push_err(OxiError::forbidden(format!(
                    "services is forbidden in {namespace}"
                )));
        } else {
            let items = objects
                .iter()
                .filter(|o| o.namespace() == Some(namespace))
                .cloned()
                .collect();
            resources.script().list.push_ok(ListPage::complete(items));
        }
    }
}

#[tokio::test]
async fn a_forbidden_list_is_a_failure_that_suggests_manual_configuration() {
    let detector = ProviderDetector::new();
    let resources = Arc::new(FakeResourcePort::new());
    let all: Vec<&str> = detector.catalogue().namespaces().into_iter().collect();
    script_lists(&resources, &detector, &all, &[]);
    let result = detector
        .detect(resources.clone(), Arc::new(FakeServiceProxyPort::new()))
        .await;
    let DetectionState::Failed(failure) = &result.state else {
        panic!("{:?}", result.state);
    };
    assert!(failure.is_forbidden(), "{failure:?}");
    assert!(failure.reason().contains("configure the endpoint by hand"));
    assert!(
        result
            .probes
            .iter()
            .all(|p| matches!(p.outcome, ProbeOutcome::Skipped(_))),
        "nothing could be checked: {:?}",
        result.probes
    );
}

#[tokio::test]
async fn forbidden_in_one_namespace_does_not_hide_a_provider_found_in_another() {
    let detector = ProviderDetector::new();
    let resources = Arc::new(FakeResourcePort::new());
    let objects = [named("monitoring", "prometheus-operated", "web", 9090)];
    script_lists(&resources, &detector, &["openshift-monitoring"], &objects);
    let backend = Backend {
        namespace: "monitoring",
        name_prefix: "prometheus-operated",
        path: "/api/v1/query",
    };
    let result = detector.detect(resources, proxy(vec![backend])).await;
    assert_eq!(found(&result).kind, MetricsProviderKind::KubePrometheus);
    assert!(
        result
            .probes
            .iter()
            .any(|p| p.namespace == "openshift-monitoring"
                && matches!(p.outcome, ProbeOutcome::Skipped(_)))
    );
}

#[tokio::test]
async fn absence_cannot_be_asserted_past_a_forbidden_namespace() {
    let detector = ProviderDetector::new();
    let resources = Arc::new(FakeResourcePort::new());
    script_lists(&resources, &detector, &["monitoring"], &[]);
    let result = detector
        .detect(resources, Arc::new(FakeServiceProxyPort::new()))
        .await;
    let DetectionState::Failed(DetectionFailure::Forbidden(detail)) = &result.state else {
        panic!("{:?}", result.state);
    };
    assert!(detail.contains("monitoring"), "{detail}");
}

#[tokio::test]
async fn another_list_failure_is_reported_with_its_namespace() {
    let detector = ProviderDetector::new();
    let resources = Arc::new(FakeResourcePort::new());
    for _ in detector.catalogue().namespaces() {
        resources
            .script()
            .list
            .push_err(OxiError::network("connection reset"));
    }
    let result = detector
        .detect(resources, Arc::new(FakeServiceProxyPort::new()))
        .await;
    let DetectionState::Failed(DetectionFailure::Other(detail)) = &result.state else {
        panic!("{:?}", result.state);
    };
    assert!(detail.contains("connection reset"), "{detail}");
}

#[tokio::test]
async fn a_name_match_whose_probe_fails_is_a_failure_naming_the_service_and_the_reason() {
    let result = detect(
        &ProviderDetector::new(),
        vec![named("monitoring", "prometheus-operated", "web", 9090)],
        // No backend: the API server answers 503 "no endpoints available".
        Arc::new(FakeServiceProxyPort::new()),
    )
    .await;
    let DetectionState::Failed(DetectionFailure::ProbeFailed {
        kind,
        target,
        reason,
    }) = &result.state
    else {
        panic!("{:?}", result.state);
    };
    assert_eq!(*kind, MetricsProviderKind::KubePrometheus);
    assert_eq!(target.to_string(), "monitoring/prometheus-operated:web");
    assert!(reason.contains("no ready endpoint"), "{reason}");
    assert!(
        result.probes.iter().any(
            |p| matches!(&p.outcome, ProbeOutcome::Failed(r) if r.contains("no ready endpoint"))
        )
    );
}

#[tokio::test]
async fn a_blocked_service_proxy_says_so() {
    let fake = Arc::new(FakeServiceProxyPort::new());
    fake.respond_with(|_, _| {
        Some(Ok(FakeServiceProxyPort::api_error(
            403,
            "services/proxy is forbidden: User cannot get resource",
        )))
    });
    let result = detect(
        &ProviderDetector::new(),
        vec![named("monitoring", "prometheus-operated", "web", 9090)],
        fake.clone(),
    )
    .await;
    let DetectionState::Failed(DetectionFailure::ProbeFailed { reason, .. }) = &result.state else {
        panic!("{:?}", result.state);
    };
    assert!(reason.contains("service proxy"), "{reason}");
    assert_eq!(
        fake.recorded_calls().len(),
        1,
        "a refused proxy is not retried under another prefix"
    );
}

#[tokio::test]
async fn the_first_catalogue_entry_that_answers_wins_and_the_rest_are_not_probed() {
    let fake = proxy(vec![
        Backend {
            namespace: "monitoring",
            name_prefix: "kube-prometheus-stack-prometheus",
            path: "/api/v1/query",
        },
        Backend {
            namespace: "monitoring",
            name_prefix: "prometheus-operated",
            path: "/api/v1/query",
        },
    ]);
    let result = detect(
        &ProviderDetector::new(),
        vec![
            named("monitoring", "prometheus-operated", "web", 9090),
            named(
                "monitoring",
                "kube-prometheus-stack-prometheus",
                "http-web",
                9090,
            ),
        ],
        fake.clone(),
    )
    .await;
    assert_eq!(
        service_of(&result),
        "monitoring/kube-prometheus-stack-prometheus:http-web"
    );
    let operated = result
        .probes
        .iter()
        .find(|p| p.service == "prometheus-operated" && p.namespace == "monitoring")
        .unwrap();
    assert_eq!(operated.outcome, ProbeOutcome::NotNeeded);
    assert_eq!(fake.recorded_calls().len(), 1, "one probe query in total");
}

#[tokio::test]
async fn a_failed_candidate_does_not_stop_a_later_one_from_winning() {
    let fake = proxy(vec![Backend {
        namespace: "monitoring",
        name_prefix: "prometheus-operated",
        path: "/api/v1/query",
    }]);
    let result = detect(
        &ProviderDetector::new(),
        vec![
            // Matches the first entry but nothing answers behind it.
            named(
                "monitoring",
                "kube-prometheus-stack-prometheus",
                "http-web",
                9090,
            ),
            named("monitoring", "prometheus-operated", "web", 9090),
        ],
        fake,
    )
    .await;
    assert_eq!(service_of(&result), "monitoring/prometheus-operated:web");
    assert!(
        result
            .probes
            .iter()
            .any(|p| matches!(p.outcome, ProbeOutcome::Failed(_)))
    );
}

#[tokio::test]
async fn ports_are_chosen_by_preference_then_the_only_port() {
    let backend = || Backend {
        namespace: "monitoring",
        name_prefix: "prometheus-operated",
        path: "/api/v1/query",
    };
    // Two ports, the preferred name is there: it wins over the first listed.
    let two = service(
        "monitoring",
        "prometheus-operated",
        json!([{ "name": "grpc", "port": 10901 }, { "name": "web", "port": 9090 }]),
    );
    let result = detect(&ProviderDetector::new(), vec![two], proxy(vec![backend()])).await;
    assert_eq!(service_of(&result), "monitoring/prometheus-operated:web");
    // One port with an unexpected name and number: the only port is used, by number.
    let odd = service(
        "monitoring",
        "prometheus-operated",
        json!([{ "name": "x", "port": 7777 }]),
    );
    let result = detect(&ProviderDetector::new(), vec![odd], proxy(vec![backend()])).await;
    assert_eq!(service_of(&result), "monitoring/prometheus-operated:7777");
    // Several ports and none expected: no usable port, and the probe list says so.
    let none = service(
        "monitoring",
        "prometheus-operated",
        json!([{ "name": "a", "port": 1 }, { "name": "b", "port": 2 }]),
    );
    let result = detect(&ProviderDetector::new(), vec![none], proxy(vec![backend()])).await;
    assert_eq!(result.state, DetectionState::NotFound);
    assert!(
        result
            .probes
            .iter()
            .any(|p| p.outcome == ProbeOutcome::NoUsablePort)
    );
}

#[tokio::test]
async fn an_external_name_service_is_not_a_candidate() {
    let external = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": "prometheus-operated", "namespace": "monitoring" },
        "spec": { "type": "ExternalName", "externalName": "prom.example.com" }
    }))
    .unwrap();
    let result = detect(
        &ProviderDetector::new(),
        vec![external],
        Arc::new(FakeServiceProxyPort::new()),
    )
    .await;
    assert_eq!(result.state, DetectionState::NotFound);
}

#[tokio::test]
async fn a_catalogue_entry_added_as_data_is_detected_without_code_changes() {
    let catalogue = Catalogue::parse(
        r#"{ "entries": [ {
            "kind": "victoria_metrics", "service": "vm-custom",
            "namespaces": ["my-team"], "ports": ["9999"], "path_prefixes": ["/vm"]
        } ] }"#,
    )
    .unwrap();
    let detector = ProviderDetector::new().with_catalogue(catalogue);
    let backend = Backend {
        namespace: "my-team",
        name_prefix: "vm-custom",
        path: "/vm/api/v1/query",
    };
    let result = detect(
        &detector,
        vec![service("my-team", "vm-custom", json!([{ "port": 9999 }]))],
        proxy(vec![backend]),
    )
    .await;
    assert_eq!(found(&result).kind, MetricsProviderKind::VictoriaMetrics);
    assert_eq!(found(&result).path_prefix.as_deref(), Some("/vm"));
    assert_eq!(
        found(&result).target,
        ProviderTarget::Service(oxikube_ports::ServiceRef::new(
            "my-team",
            "vm-custom",
            ServicePort::Number(9999)
        ))
    );
}

#[tokio::test]
async fn detection_is_read_only() {
    let resources = Arc::new(FakeResourcePort::new());
    let fake = proxy(vec![]);
    let detector = ProviderDetector::new();
    let _ = detector.detect(resources.clone(), fake.clone()).await;
    assert!(resources.mutating_calls().is_empty());
    assert!(
        fake.recorded_calls()
            .iter()
            .all(|c| c.path.ends_with("/api/v1/query")),
        "only the probe query is sent"
    );
}

#[tokio::test]
async fn checking_a_service_endpoint_goes_through_the_proxy_and_a_url_goes_direct() {
    let direct = Arc::new(oxikube_testkit::FakePromqlEndpointPort::new());
    direct.script().push_ok(Vec::new());
    let detector = ProviderDetector::with_client(direct.clone());
    let fake = proxy(vec![Backend {
        namespace: "monitoring",
        name_prefix: "vmsingle",
        path: "/api/v1/query",
    }]);
    let proxied = oxikube_ports::PromEndpoint::service(oxikube_ports::ServiceRef::new(
        "monitoring",
        "vmsingle-x",
        ServicePort::Number(8429),
    ));
    let as_proxy: Arc<dyn ServiceProxyPort> = fake.clone();
    detector.check(as_proxy.clone(), &proxied).await.unwrap();
    assert_eq!(fake.recorded_calls().len(), 1);
    assert!(direct.recorded_calls().is_empty());

    let url = oxikube_ports::PromEndpoint::new("https://vm.example.com");
    detector.check(as_proxy, &url).await.unwrap();
    assert_eq!(direct.recorded_calls().len(), 1);
    assert_eq!(
        fake.recorded_calls().len(),
        1,
        "a URL never touches the proxy"
    );
}
