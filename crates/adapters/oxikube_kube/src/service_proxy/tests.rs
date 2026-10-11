//! The proxy path and request building. The live exchange is tested against kind in
//! `tests/kind_service_proxy.rs`.

use oxikube_ports::{ProxyRequest, ProxyScheme, ServicePort, ServiceRef};

use super::request::{build, proxy_path};

fn service(port: ServicePort) -> ServiceRef {
    ServiceRef::new("monitoring", "prometheus-operated", port)
}

#[test]
fn the_path_follows_the_kubernetes_documentation() {
    assert_eq!(
        proxy_path(&service(ServicePort::Number(9090)), "/api/v1/query").unwrap(),
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/query"
    );
    assert_eq!(
        proxy_path(&service(ServicePort::Name("web".into())), "/").unwrap(),
        "/api/v1/namespaces/monitoring/services/prometheus-operated:web/proxy/"
    );
    let tls = service(ServicePort::Number(9091)).with_scheme(ProxyScheme::Https);
    assert_eq!(
        proxy_path(&tls, "/api/v1/query").unwrap(),
        "/api/v1/namespaces/monitoring/services/https:prometheus-operated:9091/proxy/api/v1/query"
    );
}

#[test]
fn names_and_paths_that_could_change_the_route_are_refused() {
    let ok = service(ServicePort::Number(80));
    for path in [
        "api/v1", "/a/../b", "/a?x=1", "/a#b", "/a b", "//x/./y", "/a\\b", "",
    ] {
        assert!(proxy_path(&ok, path).is_err(), "{path:?}");
    }
    for (namespace, name) in [
        ("a/b", "svc"),
        ("ns", "svc/../x"),
        ("ns", ""),
        ("n s", "svc"),
        ("ns", "svc:80"),
    ] {
        let bad = ServiceRef::new(namespace, name, ServicePort::Number(80));
        assert!(proxy_path(&bad, "/").is_err(), "{namespace}/{name}");
    }
    let bad_port = service(ServicePort::Name("w/eb".into()));
    assert!(proxy_path(&bad_port, "/").is_err());
}

#[test]
fn parameters_go_in_the_query_or_the_form_encoded() {
    let svc = service(ServicePort::Number(9090));
    let mut get = ProxyRequest::get("/api/v1/query");
    get.params = vec![("query".into(), "sum(rate(x[5m])) by (pod)".into())];
    let built = build(&svc, &get).unwrap();
    assert_eq!(built.method(), http::Method::GET);
    assert_eq!(
        built.uri().to_string(),
        "/api/v1/namespaces/monitoring/services/prometheus-operated:9090/proxy/api/v1/query?query=sum%28rate%28x%5B5m%5D%29%29+by+%28pod%29"
    );

    let mut post = ProxyRequest::get("/api/v1/query");
    post.method = oxikube_ports::ProxyMethod::PostForm;
    post.params = vec![("query".into(), "up".into())];
    let built = build(&svc, &post).unwrap();
    assert_eq!(built.method(), http::Method::POST);
    assert!(built.uri().query().is_none());
    assert_eq!(
        built.headers()[http::header::CONTENT_TYPE],
        "application/x-www-form-urlencoded"
    );
}
