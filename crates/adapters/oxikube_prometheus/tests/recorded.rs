//! The client against responses recorded from real servers (see `tests/fixtures/README.md`):
//! Prometheus 3.5.0, VictoriaMetrics 1.122.0 and Mimir 2.17.0.

mod common;

use common::{endpoint, fixture, range};
use oxikube_ports::{PromqlEndpointPort, PromqlError};
use oxikube_prometheus::PromqlClient;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `(fixture prefix, path prefix the server serves under, a metric the fixture was recorded for)`.
const PROVIDERS: [(&str, &str, &str); 3] = [
    ("prometheus-3.5.0", "", "up"),
    ("victoriametrics-1.122.0", "", "vm_app_version"),
    ("mimir-2.17.0", "/prometheus", "up"),
];

async fn serve(
    server: &MockServer,
    route: &str,
    params: &[(&str, &str)],
    status: u16,
    body: String,
) {
    let mut mock = Mock::given(method("GET")).and(path(route));
    for (k, v) in params {
        mock = mock.and(query_param(*k, *v));
    }
    mock.respond_with(
        ResponseTemplate::new(status).set_body_raw(body.into_bytes(), "application/json"),
    )
    .mount(server)
    .await;
}

#[tokio::test]
async fn instant_vector_from_every_provider() {
    for (name, prefix, metric) in PROVIDERS {
        let server = MockServer::start().await;
        serve(
            &server,
            &format!("{prefix}/api/v1/query"),
            &[("query", metric)],
            200,
            fixture(&format!("{name}-instant-vector")),
        )
        .await;
        let ep = endpoint(&server).with_path_prefix(prefix);
        let series = PromqlClient::new()
            .instant_query(&ep, metric, None)
            .await
            .unwrap();
        assert_eq!(series.len(), 1, "{name}");
        assert_eq!(series[0].points.len(), 1, "{name}");
        assert_eq!(series[0].points[0].value, 1.0, "{name}");
        assert_eq!(
            series[0].labels.get("__name__").map(String::as_str),
            Some(metric),
            "{name}"
        );
        assert!(series[0].labels.contains_key("job"), "{name}");
    }
}

#[tokio::test]
async fn range_matrix_from_every_provider() {
    for (name, prefix, metric) in PROVIDERS {
        let server = MockServer::start().await;
        serve(
            &server,
            &format!("{prefix}/api/v1/query_range"),
            &[
                ("query", metric),
                ("start", "1791707595.000"),
                ("end", "1791707599.000"),
                ("step", "2"),
            ],
            200,
            fixture(&format!("{name}-range-matrix")),
        )
        .await;
        let ep = endpoint(&server).with_path_prefix(prefix);
        let series = PromqlClient::new()
            .range_query(&ep, metric, range())
            .await
            .unwrap();
        assert_eq!(series.len(), 1, "{name}");
        let points = &series[0].points;
        assert!(points.len() >= 3, "{name}: {points:?}");
        assert!(
            points.windows(2).all(|w| w[0].ts < w[1].ts),
            "{name}: ascending"
        );
        assert!(points.iter().all(|p| p.value == 1.0), "{name}");
    }
}

#[tokio::test]
async fn scalar_empty_nan_and_inf_results() {
    for (name, prefix, _) in PROVIDERS {
        let server = MockServer::start().await;
        let route = format!("{prefix}/api/v1/query");
        for (query, case) in [
            ("1+1", "instant-scalar"),
            ("oxikube_no_such_metric", "instant-empty"),
            (
                "vector(1/0) or label_replace(vector(-1/0),\"s\",\"neg\",\"\",\"\")",
                "instant-inf",
            ),
        ] {
            serve(
                &server,
                &route,
                &[("query", query)],
                200,
                fixture(&format!("{name}-{case}")),
            )
            .await;
        }
        let ep = endpoint(&server).with_path_prefix(prefix);
        let client = PromqlClient::new();

        let scalar = client.instant_query(&ep, "1+1", None).await.unwrap();
        assert_eq!(scalar.len(), 1, "{name}");
        assert!(scalar[0].labels.is_empty(), "{name}");
        assert_eq!(scalar[0].points[0].value, 2.0, "{name}");

        let empty = client
            .instant_query(&ep, "oxikube_no_such_metric", None)
            .await
            .unwrap();
        assert!(
            empty.is_empty(),
            "{name}: no data is Ok(empty), not an error"
        );

        let inf = client
            .instant_query(
                &ep,
                "vector(1/0) or label_replace(vector(-1/0),\"s\",\"neg\",\"\",\"\")",
                None,
            )
            .await
            .unwrap();
        let mut values: Vec<f64> = inf.iter().map(|s| s.points[0].value).collect();
        // Servers order a vector differently; the values are what matter.
        values.sort_by(f64::total_cmp);
        assert_eq!(values, [f64::NEG_INFINITY, f64::INFINITY], "{name}");
    }
}

#[tokio::test]
async fn nan_samples_are_kept_as_nan() {
    for (name, prefix) in [("prometheus-3.5.0", ""), ("mimir-2.17.0", "/prometheus")] {
        let server = MockServer::start().await;
        serve(
            &server,
            &format!("{prefix}/api/v1/query"),
            &[("query", "vector(0/0)")],
            200,
            fixture(&format!("{name}-instant-nan")),
        )
        .await;
        let ep = endpoint(&server).with_path_prefix(prefix);
        let series = PromqlClient::new()
            .instant_query(&ep, "vector(0/0)", None)
            .await
            .unwrap();
        assert!(series[0].points[0].value.is_nan(), "{name}");
    }
}

#[tokio::test]
async fn recorded_error_responses_map_to_the_taxonomy() {
    // (fixture, http status the server used, query sent)
    let cases = [
        ("prometheus-3.5.0-error-bad-data", 400, "up{"),
        ("mimir-2.17.0-error-bad-data", 400, "up{"),
        ("victoriametrics-1.122.0-error-bad-data", 422, "up{"),
        ("prometheus-3.5.0-error-execution", 422, "dup"),
        ("mimir-2.17.0-error-execution", 422, "dup"),
    ];
    for (name, status, query) in cases {
        let server = MockServer::start().await;
        serve(
            &server,
            "/api/v1/query",
            &[("query", query)],
            status,
            fixture(name),
        )
        .await;
        let err = PromqlClient::new()
            .instant_query(&endpoint(&server), query, None)
            .await
            .unwrap_err();
        match err {
            PromqlError::BadQuery { message } => {
                assert!(
                    !message.is_empty(),
                    "{name}: the server's explanation is kept"
                );
            }
            other => panic!("{name}: expected BadQuery, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn recorded_range_error_is_a_bad_query() {
    for name in ["prometheus-3.5.0", "mimir-2.17.0"] {
        let server = MockServer::start().await;
        serve(
            &server,
            "/api/v1/query_range",
            &[],
            400,
            fixture(&format!("{name}-error-bad-range")),
        )
        .await;
        let err = PromqlClient::new()
            .range_query(&endpoint(&server), "up", range())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, PromqlError::BadQuery { message } if message.contains("end timestamp")),
            "{err:?}"
        );
    }
}
