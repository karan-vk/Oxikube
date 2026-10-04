//! `list_all` pagination and the 410 restart.

use oxikube_domain::ErrorKind;
use oxikube_ports::ListOptions;

use super::harness::*;
use crate::resources::{ResourcesConfig, is_list_expired};

#[tokio::test]
async fn list_all_follows_continue_tokens_until_exhausted() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a", "b"], Some("t1"), "100"));
    api.reply(PODS, 200, pod_list(&["c", "d"], Some("t2"), "100"));
    api.reply(PODS, 200, pod_list(&["e"], None, "100"));

    let all = resources(&api)
        .list_all(
            &pod_gvk(),
            Some("default"),
            &ListOptions::default().limit(2),
        )
        .await
        .unwrap();

    let names: Vec<_> = all.items.iter().map(|r| r.name().to_owned()).collect();
    assert_eq!(names, ["a", "b", "c", "d", "e"]);
    assert_eq!(all.continue_token, None);
    assert_eq!(all.resource_version.as_deref(), Some("100"));
    assert_eq!(api.hits(PODS), 3);
    assert_eq!(query_of(&api, PODS, 0), "limit=2");
    assert_eq!(query_of(&api, PODS, 1), "limit=2&continue=t1");
    assert_eq!(query_of(&api, PODS, 2), "limit=2&continue=t2");
}

#[tokio::test]
async fn list_all_uses_the_configured_page_size_by_default() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a"], None, "1"));
    let config = ResourcesConfig {
        page_size: 77,
        ..ResourcesConfig::default()
    };
    resources_with(&api, config)
        .list_all(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(query_of(&api, PODS, 0), "limit=77");
}

#[tokio::test]
async fn an_empty_continue_token_ends_the_list() {
    let api = server();
    let mut page = pod_list(&["a"], None, "5");
    page["metadata"]["continue"] = "".into();
    api.reply(PODS, 200, page);
    let all = resources(&api)
        .list_all(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap();
    assert_eq!(all.items.len(), 1);
    assert_eq!(api.hits(PODS), 1);
}

#[tokio::test]
async fn a_stale_continue_token_restarts_the_list_from_the_first_page() {
    let api = server();
    api.reply(PODS, 200, pod_list(&["a", "b"], Some("old"), "100"));
    api.reply(PODS, 410, gone());
    api.reply(PODS, 200, pod_list(&["a", "b"], Some("new"), "200"));
    api.reply(PODS, 200, pod_list(&["c"], None, "200"));

    let all = resources(&api)
        .list_all(
            &pod_gvk(),
            Some("default"),
            &ListOptions::default().limit(2),
        )
        .await
        .unwrap();

    // The first attempt's items are discarded: no duplicates, one snapshot.
    let names: Vec<_> = all.items.iter().map(|r| r.name().to_owned()).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(all.resource_version.as_deref(), Some("200"));
    assert_eq!(api.hits(PODS), 4);
    assert_eq!(
        query_of(&api, PODS, 2),
        "limit=2",
        "restart begins at page one"
    );
}

#[tokio::test]
async fn restarts_are_bounded() {
    let api = server();
    for _ in 0..4 {
        api.reply(PODS, 200, pod_list(&["a"], Some("t"), "1"));
        api.reply(PODS, 410, gone());
    }
    let err = resources(&api)
        .list_all(
            &pod_gvk(),
            Some("default"),
            &ListOptions::default().limit(1),
        )
        .await
        .unwrap_err();
    assert!(is_list_expired(&err), "{err:?}");
    // max_restarts (3) + the first attempt = 4 first-pages, each followed by a 410.
    assert_eq!(api.hits(PODS), 8);
}

#[tokio::test]
async fn gone_on_the_first_page_is_not_retried() {
    let api = server();
    api.reply(PODS, 410, gone());
    let err = resources(&api)
        .list_all(&pod_gvk(), Some("default"), &ListOptions::default())
        .await
        .unwrap_err();
    assert!(is_list_expired(&err));
    assert_eq!(api.hits(PODS), 1);
}

#[tokio::test]
async fn a_single_page_list_surfaces_the_expired_token() {
    let api = server();
    api.reply(PODS, 410, gone());
    let err = oxikube_ports::ResourceReader::list(
        &resources(&api),
        &pod_gvk(),
        Some("default"),
        &ListOptions::default().continue_from("stale"),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(is_list_expired(&err));
    assert!(err.is_retryable());
}

#[tokio::test]
async fn list_all_rejects_a_continue_token() {
    let api = server();
    let err = resources(&api)
        .list_all(
            &pod_gvk(),
            Some("default"),
            &ListOptions::default().continue_from("x"),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_eq!(api.hits(PODS), 0);
}
