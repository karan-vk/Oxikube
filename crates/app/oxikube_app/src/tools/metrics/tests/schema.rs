//! Input validation: a malformed call is an `Err` the model can correct, before anything runs.

use oxikube_domain::ErrorKind;
use serde_json::json;

use super::*;
use crate::tools::metrics::{QUERY_PROMQL, TOP_NODES, TOP_PODS};

async fn rejected(tool: &str, args: Value) -> String {
    // Nothing is connected: a malformed call is refused before a cluster is looked for.
    let h = Harness::new();
    let error = h.call(tool, args).await.expect_err("a validation error");
    assert_eq!(error.kind(), ErrorKind::Validation, "{}", error.message());
    error.message().to_owned()
}

#[tokio::test]
async fn a_bad_sort_key_or_limit_is_refused() {
    for tool in [TOP_PODS, TOP_NODES] {
        let message = rejected(tool, json!({"sort": "disk"})).await;
        assert!(message.contains("sort"), "{message}");
        for limit in [json!(-1), json!(0), json!(51), json!("ten"), json!(1.5)] {
            rejected(tool, json!({"limit": limit})).await;
        }
        rejected(tool, json!({"nope": 1})).await;
    }
    rejected(TOP_PODS, json!({"namespace": ""})).await;
}

#[tokio::test]
async fn query_promql_needs_exactly_one_form() {
    rejected(QUERY_PROMQL, json!({"query": ""})).await;
    let blank = rejected(QUERY_PROMQL, json!({"query": "   "})).await;
    assert!(blank.contains("empty"), "{blank}");
    let neither = rejected(QUERY_PROMQL, json!({})).await;
    assert!(neither.contains("query"), "{neither}");
    let both = rejected(
        QUERY_PROMQL,
        json!({"query": "up", "scope": "cluster", "metric": "cpu_usage"}),
    )
    .await;
    assert!(both.contains("not both"), "{both}");
    let long = "a".repeat(5_000);
    rejected(QUERY_PROMQL, json!({"query": long})).await;
}

#[tokio::test]
async fn a_catalogue_selector_is_checked_before_it_runs() {
    let unknown_scope = rejected(
        QUERY_PROMQL,
        json!({"scope": "galaxy", "metric": "cpu_usage"}),
    )
    .await;
    assert!(
        unknown_scope.contains("\"cluster\", \"node\""),
        "{unknown_scope}"
    );
    rejected(QUERY_PROMQL, json!({"scope": "cluster", "metric": "joy"})).await;
    let no_name = rejected(
        QUERY_PROMQL,
        json!({"scope": "node", "metric": "cpu_usage"}),
    )
    .await;
    assert!(no_name.contains("`name` is required"), "{no_name}");
    let no_ns = rejected(
        QUERY_PROMQL,
        json!({"scope": "pod", "metric": "cpu_usage", "name": "web-0"}),
    )
    .await;
    assert!(no_ns.contains("`namespace` is required"), "{no_ns}");
    let unsupported = rejected(
        QUERY_PROMQL,
        json!({"scope": "pod", "metric": "pod_count", "namespace": "a", "name": "b"}),
    )
    .await;
    assert!(unsupported.contains("pod_count"), "{unsupported}");
    rejected(
        QUERY_PROMQL,
        json!({"scope": "cluster", "metric": "cpu_usage", "labels": {"bad name": "x"}}),
    )
    .await;
    for range in ["soon", "30s", "8d", "0m"] {
        rejected(QUERY_PROMQL, json!({"query": "up", "range": range})).await;
    }
}
