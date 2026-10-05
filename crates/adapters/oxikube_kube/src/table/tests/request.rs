//! Request building: the Table `Accept` header and `includeObject` on kube's URLs.

use kube::api::{ListParams, WatchParams};
use oxikube_ports::IncludeObject;

use crate::table::TABLE_ACCEPT;
use crate::table::request::{list, watch};

fn accept(request: &http::Request<Vec<u8>>) -> &str {
    request.headers()[http::header::ACCEPT].to_str().unwrap()
}

#[test]
fn a_bare_list_gets_include_object_as_its_only_parameter() {
    let request = list(
        "/api/v1/pods",
        &ListParams::default(),
        IncludeObject::Metadata,
    )
    .unwrap();
    assert_eq!(request.uri(), "/api/v1/pods?includeObject=Metadata");
    assert_eq!(accept(&request), TABLE_ACCEPT);
    assert_eq!(request.method(), http::Method::GET);
}

/// kube's query serializer starts from `path?`, so a query with pairs reads `?&key=...`;
/// the apiserver ignores the empty pair.
#[test]
fn list_parameters_are_kept_and_include_object_appended() {
    let params = ListParams::default().labels("a=b").limit(5);
    let request = list("/api/v1/namespaces/x/pods", &params, IncludeObject::Object).unwrap();
    assert_eq!(
        request.uri(),
        "/api/v1/namespaces/x/pods?&labelSelector=a%3Db&limit=5&includeObject=Object"
    );
}

#[test]
fn a_watch_carries_the_version_and_the_table_accept_header() {
    let request = watch(
        "/apis/g/v1/widgets",
        &WatchParams::default().timeout(10),
        "42",
        IncludeObject::None,
    )
    .unwrap();
    let uri = request.uri().to_string();
    assert!(
        uri.starts_with("/apis/g/v1/widgets?&watch=true&timeoutSeconds=10"),
        "{uri}"
    );
    assert!(
        uri.ends_with("&resourceVersion=42&includeObject=None"),
        "{uri}"
    );
    assert_eq!(accept(&request), TABLE_ACCEPT);
}

#[test]
fn an_invalid_parameter_is_a_validation_error() {
    let err = watch(
        "/api/v1/pods",
        &WatchParams::default().timeout(10_000),
        "1",
        IncludeObject::Metadata,
    )
    .unwrap_err();
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::Validation);
}
