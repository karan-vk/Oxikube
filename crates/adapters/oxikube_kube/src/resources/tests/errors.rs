//! Error taxonomy: server failures and scope/verb checks.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_ports::{ListOptions, ResourceReader};

use super::harness::*;
use crate::fake_api::status_body;

const POD_A: &str = "/api/v1/namespaces/default/pods/a";

#[tokio::test]
async fn forbidden_list_keeps_the_servers_explanation() {
    let api = server();
    let message = "pods is forbidden: User \"dev\" cannot list resource \"pods\" in API group \"\" in the namespace \"default\"";
    api.reply(PODS, 403, status_body(403, "Forbidden", message));
    let err = resources(&api)
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("cannot list resource"), "{err}");
}

#[tokio::test]
async fn unauthorized_and_throttled_lists_map_to_auth_and_network() {
    let api = server();
    api.reply(PODS, 401, status_body(401, "Unauthorized", "Unauthorized"));
    let r = resources(&api);
    let err = list_pods(&r, ListOptions::default()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Auth);

    let api = server();
    api.reply(PODS, 429, status_body(429, "TooManyRequests", "slow down"));
    let r = resources(&api);
    let err = r
        .list(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Network);
    assert!(err.is_retryable());
}

#[tokio::test]
async fn get_of_a_missing_object_is_not_found_and_get_opt_is_none() {
    let api = server();
    api.reply(
        POD_A,
        404,
        status_body(404, "NotFound", "pods \"a\" not found"),
    );
    let r = resources(&api);

    let err = r.get(&pod_gvk(), Some("default"), "a").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("default/a"), "{err}");

    assert!(
        r.get_opt(&pod_gvk(), Some("default"), "a")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn get_failures_other_than_404_are_not_swallowed() {
    let api = server();
    api.reply(POD_A, 403, status_body(403, "Forbidden", "nope"));
    let r = resources(&api);
    assert_eq!(
        r.get_opt(&pod_gvk(), Some("default"), "a")
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Forbidden
    );
}

#[tokio::test]
async fn a_kind_the_cluster_does_not_serve_is_unsupported() {
    let api = server();
    let err = resources(&api)
        .list(
            &Gvk::new("nope.io", "v1", "Ghost"),
            None,
            &ListOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(err.message().contains("nope.io/v1/Ghost"), "{err}");
}

#[tokio::test]
async fn a_kind_without_the_verb_is_unsupported() {
    let api = server();
    let binding = Gvk::new("", "v1", "Binding");
    let err = resources(&api)
        .list(&binding, Some("default"), &ListOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let err = resources(&api)
        .get(&binding, Some("default"), "b")
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn get_checks_the_namespace_against_the_scope() {
    let api = server();
    let r = resources(&api);
    let missing = r.get(&pod_gvk(), None, "a").await.unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::Validation);
    let extra = r
        .get(&namespace_gvk(), Some("default"), "x")
        .await
        .unwrap_err();
    assert_eq!(extra.kind(), ErrorKind::Validation);
    assert_eq!(api.hits(POD_A), 0);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_list_times_out_at_the_deadline() {
    let api = server();
    api.stall(PODS);
    let r = resources(&api);
    let started = tokio::time::Instant::now();
    let err = list_pods(&r, ListOptions::default().timeout_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
    assert!(err.is_retryable());
    assert_eq!(started.elapsed(), std::time::Duration::from_secs(5));
    assert_eq!(api.hits(PODS), 1);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_metadata_list_and_paged_list_time_out_too() {
    let api = server();
    api.stall(PODS);
    let r = resources(&api);
    let options = ListOptions::default().timeout_secs(2);
    let err = r
        .list_metadata(&pod_gvk(), Some("default"), &options)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
    let err = r
        .list_all(&pod_gvk(), Some("default"), &options)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
}

#[tokio::test(start_paused = true)]
async fn a_prompt_list_is_unaffected_by_a_deadline() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "7"));
    let r = resources(&api);
    let page = list_pods(&r, ListOptions::default().timeout_secs(5))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    // The deadline is not a query parameter.
    assert!(!query_of(&api, PODS, 0).contains("timeout"));
}
