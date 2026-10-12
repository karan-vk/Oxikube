//! `k8s.query_promql`: raw and catalogue queries, bounded output, and every way Prometheus can
//! be missing.

use std::collections::BTreeMap;

use oxikube_domain::OxiError;
use oxikube_domain::metric_query::{MetricKind, MetricTarget, PodSelector};
use oxikube_ports::{PromqlError, PromqlSeries, PromqlValue};
use oxikube_testkit::PromqlCall;
use serde_json::json;

use super::*;
use crate::tools::metrics::QUERY_PROMQL;

fn series(labels: &[(&str, &str)], points: &[(i64, f64)]) -> PromqlSeries {
    PromqlSeries {
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        points: points
            .iter()
            .map(|(s, v)| PromqlValue {
                ts: Timestamp::from_second(*s).unwrap(),
                value: *v,
            })
            .collect(),
    }
}

#[tokio::test]
async fn without_a_prometheus_the_answer_says_why() {
    let h = Harness::new();
    h.connect_without_provider().await;
    let out = h.call(QUERY_PROMQL, json!({"query": "up"})).await.unwrap();
    assert!(!out.is_error, "no Prometheus is a result, not an exception");
    let data = payload(&out);
    assert_eq!(data["status"], "unavailable");
    assert!(
        data["reason"]
            .as_str()
            .unwrap()
            .contains("no Prometheus-compatible service"),
        "{data}"
    );
    assert!(
        text(&out).starts_with("metrics unavailable: "),
        "{}",
        text(&out)
    );
    assert!(h.promql.recorded_calls().is_empty(), "nothing was queried");
}

#[tokio::test]
async fn metrics_turned_off_in_the_settings_reject_the_query() {
    let h = Harness::with_prefs(prometheus_off());
    h.connect().await;
    let out = h.call(QUERY_PROMQL, json!({"query": "up"})).await.unwrap();
    let data = payload(&out);
    assert_eq!(data["status"], "unavailable");
    assert!(
        data["reason"].as_str().unwrap().contains("turned off"),
        "{data}"
    );
    assert!(h.promql.recorded_calls().is_empty());
}

#[tokio::test]
async fn an_instant_query_returns_the_biggest_series_rounded_and_bounded() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::KubePrometheus)
        .await;
    let mut result: Vec<PromqlSeries> = (0..30)
        .map(|i| {
            series(
                &[("pod", &format!("web-{i:02}"))],
                &[(1_790_000_000, f64::from(i) + 0.123_456_789)],
            )
        })
        .collect();
    result.push(series(&[("pod", "nan")], &[(1_790_000_000, f64::NAN)]));
    h.promql.script().query_instant.push_ok(result);

    let out = h
        .call(QUERY_PROMQL, json!({"query": " sum by (pod) (up) "}))
        .await
        .unwrap();
    let data = payload(&out);
    assert_eq!(data["status"], "ok");
    assert_eq!(data["type"], "instant");
    assert_eq!(data["query"], "sum by (pod) (up)");
    assert_eq!(data["provider"], "kube-prometheus");
    assert_eq!(data["series_total"], 31);
    let listed = data["series"].as_array().unwrap();
    assert_eq!(listed.len(), 20, "bounded");
    assert_eq!(
        listed[0]["labels"]["pod"], "web-29",
        "biggest latest value first"
    );
    assert_eq!(
        listed[0]["latest"], 29.12,
        "rounded to four significant digits"
    );
    assert_eq!(listed[0]["points"], json!([[1_790_000_000, 29.12]]));
    assert!(
        data["notes"][0]
            .as_str()
            .unwrap()
            .contains("11 more series"),
        "{}",
        data["notes"]
    );
    assert_eq!(
        h.promql.recorded_calls(),
        [PromqlCall::QueryInstant {
            cluster: cluster(),
            query: "sum by (pod) (up)".to_owned(),
            at: None
        }]
    );
}

#[tokio::test]
async fn a_range_query_runs_over_the_span_and_thins_the_points() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::KubePrometheus)
        .await;
    let points: Vec<(i64, f64)> = (0..241)
        .map(|i| (1_790_000_000 + i * 15, f64::from(i as i32)))
        .collect();
    h.promql
        .script()
        .query_range
        .push_ok(vec![series(&[], &points)]);
    let out = h
        .call(QUERY_PROMQL, json!({"query": "node_load1", "range": "1h"}))
        .await
        .unwrap();
    let data = payload(&out);
    assert_eq!(data["type"], "range");
    assert_eq!(data["range_seconds"], 3600);
    let kept = data["series"][0]["points"].as_array().unwrap();
    assert!(kept.len() <= 60 && kept.len() >= 2, "{}", kept.len());
    assert_eq!(
        kept[0],
        json!([1_790_000_000, 0.0]),
        "the first point is kept"
    );
    assert_eq!(
        kept[kept.len() - 1],
        json!([1_790_003_600, 240.0]),
        "and the last"
    );
    assert!(data["notes"][0].as_str().unwrap().contains("thinned"));

    let [PromqlCall::QueryRange { range, .. }] = &h.promql.recorded_calls()[..] else {
        panic!("one range query");
    };
    assert_eq!(range.end, t0(), "ends now, on the app clock");
    assert_eq!(range.start, t0() - Duration::from_secs(3600));
    assert_eq!(range.step, Duration::from_secs(15));
}

#[tokio::test]
async fn many_series_share_a_point_budget() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::KubePrometheus)
        .await;
    let points: Vec<(i64, f64)> = (0..100).map(|i| (1_790_000_000 + i * 60, 1.0)).collect();
    let many: Vec<PromqlSeries> = (0..20)
        .map(|i| series(&[("pod", &format!("p{i}"))], &points))
        .collect();
    h.promql.script().query_range.push_ok(many);
    let out = h
        .call(QUERY_PROMQL, json!({"query": "up", "range": "6h"}))
        .await
        .unwrap();
    let total: usize = payload(&out)["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["points"].as_array().unwrap().len())
        .sum();
    assert!(total <= 400, "{total} points");
    assert!(text(&out).len() < 16 * 1024, "{} bytes", text(&out).len());
}

#[tokio::test]
async fn the_catalogue_form_builds_the_query_for_the_detected_stack() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::Lens).await;
    h.promql
        .script()
        .query_instant
        .push_ok(vec![series(&[("pod", "web-0")], &[(1_790_000_000, 0.25)])]);
    let out = h
        .call(
            QUERY_PROMQL,
            json!({"scope": "pod", "metric": "cpu_usage", "namespace": "shop", "name": "web-0"}),
        )
        .await
        .unwrap();
    let data = payload(&out);
    assert_eq!(data["unit"], "cores");
    assert_eq!(data["provider"], "Lens metrics stack");
    assert!(
        data["query"].as_str().unwrap().starts_with("fake:"),
        "{}",
        data["query"]
    );

    let calls = h.catalogue.recorded_calls();
    let [call] = &calls[..] else {
        panic!("one build")
    };
    assert_eq!(call.provider, MetricsProviderKind::Lens);
    assert_eq!(call.request.kind, MetricKind::CpuUsage);
    assert_eq!(
        call.request.target,
        MetricTarget::Pod {
            namespace: "shop".into(),
            pods: PodSelector::One("web-0".into())
        }
    );
    assert!(
        data["notes"].as_array().unwrap().is_empty(),
        "a single-cluster store has no caveat"
    );
}

#[tokio::test]
async fn a_range_catalogue_query_uses_the_step_policys_rate_window() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::KubePrometheus)
        .await;
    h.promql.script().query_range.push_ok(vec![]);
    let out = h
        .call(
            QUERY_PROMQL,
            json!({"scope": "cluster", "metric": "memory_usage", "range": "6h"}),
        )
        .await
        .unwrap();
    assert_eq!(payload(&out)["series_total"], 0);
    assert!(
        payload(&out)["notes"][0]
            .as_str()
            .unwrap()
            .contains("no series")
    );
    assert_eq!(
        h.catalogue.recorded_calls()[0].rate_window,
        Duration::from_secs(240),
        "6h is 2m steps and a 4m window"
    );
}

#[tokio::test]
async fn a_shared_store_warns_unless_the_query_is_narrowed() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::Mimir).await;
    h.promql.script().query_instant.push_ok(vec![]);
    h.promql.script().query_instant.push_ok(vec![]);
    let args = json!({"scope": "node", "metric": "cpu_usage", "name": "n1"});
    let out = h.call(QUERY_PROMQL, args).await.unwrap();
    assert!(
        payload(&out)["notes"]
            .to_string()
            .contains("several clusters")
    );

    let narrowed =
        json!({"scope": "node", "metric": "cpu_usage", "name": "n1", "labels": {"cluster": "dev"}});
    let out = h.call(QUERY_PROMQL, narrowed).await.unwrap();
    assert!(
        !payload(&out)["notes"]
            .to_string()
            .contains("several clusters")
    );
    let calls = h.catalogue.recorded_calls();
    assert_eq!(calls[1].request.extra.len(), 1);
    assert_eq!(calls[1].request.extra[0].name(), "cluster");
}

#[tokio::test]
async fn a_query_prometheus_rejects_is_an_error_the_model_can_fix() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::KubePrometheus)
        .await;
    h.promql
        .script()
        .query_instant
        .push_err(OxiError::from(PromqlError::BadQuery {
            message: "parse error: unexpected end of input".into(),
        }));
    let out = h
        .call(QUERY_PROMQL, json!({"query": "sum("}))
        .await
        .unwrap();
    assert!(out.is_error);
    assert!(text(&out).contains("rejected the query"), "{}", text(&out));
    assert!(text(&out).contains("unexpected end of input"));
}

#[tokio::test]
async fn an_unreachable_or_forbidden_prometheus_is_unavailable_not_an_error() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::KubePrometheus)
        .await;
    for error in [
        OxiError::from(PromqlError::Unreachable("connection refused".into())),
        OxiError::from(PromqlError::Timeout(Duration::from_secs(10))),
        OxiError::from(PromqlError::Forbidden("403".into())),
        OxiError::unsupported("no prometheus endpoint is configured for this cluster"),
    ] {
        h.promql.script().query_instant.push_err(error);
        let out = h.call(QUERY_PROMQL, json!({"query": "up"})).await.unwrap();
        assert!(!out.is_error, "{}", text(&out));
        assert_eq!(payload(&out)["status"], "unavailable");
        assert!(text(&out).starts_with("metrics unavailable: "));
    }
}

#[tokio::test]
async fn secrets_in_labels_and_errors_never_reach_the_model() {
    let h = Harness::new();
    h.connect_with_provider(MetricsProviderKind::KubePrometheus)
        .await;
    h.promql.script().query_instant.push_ok(vec![series(
        &[
            ("job", "scrape"),
            ("note", "Authorization: Bearer abc123.def456.ghi789"),
        ],
        &[(1_790_000_000, 1.0)],
    )]);
    let out = h
        .call(
            QUERY_PROMQL,
            json!({"query": "up{token=\"Bearer s3cr3tvalue123\"}"}),
        )
        .await
        .unwrap();
    let everything = format!("{} {}", text(&out), payload(&out));
    for leaked in ["abc123.def456", "s3cr3tvalue123"] {
        assert!(
            !everything.contains(leaked),
            "{leaked} leaked: {everything}"
        );
    }

    h.promql
        .script()
        .query_instant
        .push_err(OxiError::from(PromqlError::Unauthorized(
            "rejected Bearer s3cr3tvalue123".into(),
        )));
    let out = h.call(QUERY_PROMQL, json!({"query": "up"})).await.unwrap();
    assert!(!text(&out).contains("s3cr3tvalue123"), "{}", text(&out));
}
