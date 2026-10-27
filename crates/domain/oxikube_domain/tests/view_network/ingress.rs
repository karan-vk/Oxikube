//! Ingress rule flattening and IngressClass.

use oxikube_domain::view::network::{
    BackendPort, DEFAULT_CLASS_ANNOTATION, IngressBackend, IngressClassSummary, IngressSummary,
    selector_text,
};
use oxikube_domain::{Resource, ViewError};
use serde_json::json;

use super::object;

fn ingress(body: serde_json::Value) -> IngressSummary {
    IngressSummary::from_resource(&object(
        "networking.k8s.io/v1",
        "Ingress",
        Some("shop"),
        "web",
        body,
    ))
    .expect("an ingress")
}

fn service(name: &str, port: serde_json::Value) -> serde_json::Value {
    json!({"service": {"name": name, "port": port}})
}

#[test]
fn rules_are_flattened_to_one_row_per_host_and_path_with_the_tls_secret() {
    let summary = ingress(json!({
        "spec": {
            "ingressClassName": "nginx",
            "tls": [
                {"hosts": ["shop.example.com"], "secretName": "shop-tls"},
                {"hosts": ["*.example.com"], "secretName": "wild-tls"}
            ],
            "rules": [
                {"host": "shop.example.com", "http": {"paths": [
                    {"path": "/api", "pathType": "Prefix", "backend": service("api", json!({"number": 8080}))},
                    {"path": "/", "pathType": "Exact", "backend": service("web", json!({"name": "http"}))}
                ]}},
                {"host": "blog.example.com", "http": {"paths": [
                    {"backend": {"resource": {"apiGroup": "k8s.example.com", "kind": "Bucket", "name": "assets"}}}
                ]}},
                {"host": "plain.test", "http": {"paths": [
                    {"path": "/", "pathType": "Prefix", "backend": service("web", json!({"number": 80}))}
                ]}}
            ]
        },
        "status": {"loadBalancer": {"ingress": [{"ip": "203.0.113.10"}, {"hostname": "lb.example.com"}]}}
    }));
    assert_eq!(summary.class.as_deref(), Some("nginx"));
    assert_eq!(summary.load_balancers, ["203.0.113.10", "lb.example.com"]);
    assert_eq!(summary.rules.len(), 4);

    let api = &summary.rules[0];
    assert_eq!(api.host.as_deref(), Some("shop.example.com"));
    assert_eq!(api.path.as_deref(), Some("/api"));
    assert_eq!(api.path_type.as_deref(), Some("Prefix"));
    assert_eq!(
        api.backend.as_ref().map(ToString::to_string).as_deref(),
        Some("api:8080")
    );
    assert_eq!(
        api.tls_secret.as_deref(),
        Some("shop-tls"),
        "an exact host wins"
    );

    let named = summary.rules[1].backend.as_ref().expect("backend");
    assert_eq!(
        named,
        &IngressBackend::Service {
            name: "web".into(),
            port: BackendPort::Name("http".into())
        }
    );
    assert_eq!(named.to_string(), "web:http");

    // A typed resource backend, a path that defaults to "/", a wildcard certificate.
    let bucket = &summary.rules[2];
    assert_eq!(bucket.path.as_deref(), Some("/"));
    assert_eq!(
        bucket.backend.as_ref().map(ToString::to_string).as_deref(),
        Some("Bucket/assets")
    );
    assert_eq!(
        bucket.backend.as_ref().and_then(IngressBackend::service),
        None
    );
    assert_eq!(bucket.tls_secret.as_deref(), Some("wild-tls"));
    assert_eq!(
        summary.rules[3].tls_secret, None,
        "no entry covers plain.test"
    );

    assert_eq!(
        summary.hosts(),
        ["shop.example.com", "blog.example.com", "plain.test"]
    );
    assert_eq!(summary.services(), ["api", "web"]);
    assert_eq!(
        summary.rules_text(),
        "shop.example.com/api → api:8080, shop.example.com/ → web:http, \
         blog.example.com/ → Bucket/assets, plain.test/ → web:80"
    );
}

#[test]
fn a_wildcard_certificate_covers_exactly_one_label() {
    let tls = json!([{"hosts": ["*.example.com"], "secretName": "wild"}]);
    let rule = |host: &str| json!({"host": host, "http": {"paths": [{"path": "/", "backend": service("s", json!({"number": 1}))}]}});
    let summary = ingress(json!({"spec": {"tls": tls, "rules": [
        rule("a.example.com"), rule("a.b.example.com"), rule("example.com")
    ]}}));
    let secrets: Vec<_> = summary
        .rules
        .iter()
        .map(|r| r.tls_secret.as_deref())
        .collect();
    assert_eq!(secrets, [Some("wild"), None, None]);
}

#[test]
fn a_rule_without_a_host_matches_every_host_and_takes_the_default_certificate() {
    let summary = ingress(json!({"spec": {
        "tls": [{"secretName": "default-tls"}],
        "rules": [{"http": {"paths": [{"path": "/x", "backend": service("s", json!({"number": 9}))}]}}],
        "defaultBackend": service("fallback", json!({"number": 80}))
    }}));
    assert_eq!(summary.rules[0].host, None);
    assert_eq!(summary.rules[0].tls_secret.as_deref(), Some("default-tls"));
    assert_eq!(summary.hosts(), ["*"]);
    assert_eq!(summary.rules_text(), "*/x → s:9, (default) → fallback:80");
    assert_eq!(summary.services(), ["s", "fallback"]);
}

#[test]
fn a_host_only_rule_keeps_its_row_and_the_legacy_class_annotation_is_read() {
    let mut res = object(
        "networking.k8s.io/v1",
        "Ingress",
        Some("shop"),
        "web",
        json!({"spec": {"rules": [{"host": "only.test"}]}}),
    );
    res.edit_json(|v| {
        v["metadata"]["annotations"] = json!({"kubernetes.io/ingress.class": "legacy"})
    });
    let res = Resource::from_json(res.to_value()).unwrap();
    let summary = IngressSummary::from_resource(&res).unwrap();
    assert_eq!(summary.class.as_deref(), Some("legacy"));
    assert_eq!(summary.rules.len(), 1);
    assert_eq!(summary.rules[0].path, None);
    assert_eq!(summary.rules[0].backend, None);
    assert_eq!(summary.rules[0].summary(), "only.test");
}

#[test]
fn an_empty_ingress_and_a_wrong_kind() {
    let summary = ingress(json!({}));
    assert!(summary.rules.is_empty() && summary.load_balancers.is_empty());
    assert_eq!(summary.rules_text(), "");
    let pod = object("v1", "Pod", Some("x"), "p", json!({}));
    assert!(matches!(
        IngressSummary::from_resource(&pod),
        Err(ViewError::WrongKind { .. })
    ));
}

fn class(annotations: serde_json::Value, spec: serde_json::Value) -> IngressClassSummary {
    let mut res = object(
        "networking.k8s.io/v1",
        "IngressClass",
        None,
        "nginx",
        json!({"spec": spec}),
    );
    res.edit_json(|v| v["metadata"]["annotations"] = annotations);
    let res = Resource::from_json(res.to_value()).expect("the annotated class is still a resource");
    IngressClassSummary::from_resource(&res).expect("an IngressClass summary")
}

#[test]
fn an_ingress_class_is_the_default_only_with_the_annotation_set_to_true() {
    let spec = json!({"controller": "k8s.io/ingress-nginx"});
    let yes = class(json!({DEFAULT_CLASS_ANNOTATION: "true"}), spec.clone());
    assert!(yes.is_default);
    assert_eq!(yes.controller.as_deref(), Some("k8s.io/ingress-nginx"));
    assert!(!class(json!({DEFAULT_CLASS_ANNOTATION: "false"}), spec.clone()).is_default);
    assert!(!class(json!({}), spec).is_default);
}

#[test]
fn ingress_class_parameters_default_to_cluster_scope() {
    let summary = class(
        json!({}),
        json!({"controller": "c", "parameters": {"apiGroup": "k8s.example.com", "kind": "IngressParameters", "name": "p"}}),
    );
    let parameters = summary.parameters.expect("parameters");
    assert_eq!(parameters.api_group.as_deref(), Some("k8s.example.com"));
    assert_eq!(
        (parameters.kind.as_str(), parameters.name.as_str()),
        ("IngressParameters", "p")
    );
    assert_eq!(parameters.scope, "Cluster");
    let namespaced = class(
        json!({}),
        json!({"parameters": {"kind": "ConfigMap", "name": "p", "scope": "Namespace", "namespace": "infra"}}),
    );
    let parameters = namespaced.parameters.expect("parameters");
    assert_eq!(
        (parameters.scope.as_str(), parameters.namespace.as_deref()),
        ("Namespace", Some("infra"))
    );
    assert_eq!(parameters.api_group, None);
}

#[test]
fn selectors_read_labels_then_expressions() {
    let selector = json!({
        "matchLabels": {"tier": "db", "app": "web"},
        "matchExpressions": [
            {"key": "env", "operator": "In", "values": ["prod", "stage"]},
            {"key": "x", "operator": "NotIn", "values": ["1"]},
            {"key": "managed", "operator": "Exists"},
            {"key": "legacy", "operator": "DoesNotExist"}
        ]
    });
    let res = object("v1", "ConfigMap", Some("a"), "c", json!({"data": selector}));
    let text = selector_text(res.json().get("data").unwrap());
    assert_eq!(
        text.as_deref(),
        Some("app=web, tier=db, env in (prod,stage), x notin (1), managed, !legacy")
    );
    let empty = object("v1", "ConfigMap", Some("a"), "c", json!({"data": {}}));
    assert_eq!(selector_text(empty.json().get("data").unwrap()), None);
}
