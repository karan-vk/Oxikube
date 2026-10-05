//! Kind integration for E04-S08: `LogPort` options, multi-container and label-selector fan-in,
//! and throughput. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly
//! otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::{Duration, Instant};

use futures::StreamExt;
use oxikube_domain::ErrorKind;
use oxikube_kube::ContainerSelection;
use oxikube_ports::{LogOptions, LogPort, LogSince};
use oxikube_testkit::integration::TestNamespace;

use common::logs::{Script, create, logs, numbers, read_lines, script_pod, wait_started};

const WITHIN: Duration = Duration::from_secs(60);

/// Prints `<prefix> 0..count` as fast as it can, then idles.
fn burst(prefix: &str, count: u32) -> String {
    format!("i=0; while [ $i -lt {count} ]; do echo \"{prefix} $i\"; i=$((i+1)); done; sleep 3600")
}

#[tokio::test]
async fn options_reach_the_server_and_lines_carry_source_and_timestamp() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    create(
        &client,
        ns.name(),
        &script_pod(
            "chatty",
            &[],
            "Never",
            &[Script {
                name: "main",
                script: &burst("n", 50),
            }],
        ),
    )
    .await;
    wait_started(&client, ns.name(), "chatty").await;

    // The only container is the default, so no name is needed.
    let all = logs
        .stream_logs(ns.name(), "chatty", &LogOptions::default())
        .await
        .expect("stream");
    let all: Vec<_> = all.map(|l| l.expect("line")).collect().await;
    assert_eq!(numbers(&all, "n"), (0..50).collect::<Vec<_>>());
    let first = &all[0];
    assert_eq!((&*first.pod, &*first.container), ("chatty", "main"));
    assert!(
        first.ts.as_second() > 1_700_000_000,
        "kubelet timestamp, not the epoch"
    );
    assert!(
        all.windows(2).all(|w| w[0].ts <= w[1].ts),
        "timestamps are ordered"
    );
    assert!(
        !first.text.contains('T'),
        "timestamp prefix is stripped from the text"
    );

    // tail
    let tail = logs
        .stream_logs(ns.name(), "chatty", &LogOptions::default().tail_lines(5))
        .await
        .expect("tail");
    let tail: Vec<_> = tail.map(|l| l.expect("line")).collect().await;
    assert_eq!(numbers(&tail, "n"), (45..50).collect::<Vec<_>>());

    // since: lines from long ago are all included, from the future none
    let recent = logs
        .stream_logs(
            ns.name(),
            "chatty",
            &LogOptions::default().since(LogSince::Seconds(3600)),
        )
        .await
        .expect("since");
    assert_eq!(recent.count().await, 50);
    let future = LogSince::Time(jiff::Timestamp::now() + jiff::SignedDuration::from_secs(3600));
    let none = logs
        .stream_logs(ns.name(), "chatty", &LogOptions::default().since(future))
        .await
        .expect("since future");
    assert_eq!(none.count().await, 0);

    // follow keeps the stream open past the last line
    let mut live = logs
        .stream_logs(ns.name(), "chatty", &LogOptions::follow())
        .await
        .expect("follow");
    let lines = read_lines(&mut live, 50, WITHIN).await;
    assert_eq!(numbers(&lines, "n").len(), 50);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), live.next())
            .await
            .is_err(),
        "a followed stream stays open while the container runs"
    );
}

#[tokio::test]
async fn errors_are_classified() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    create(
        &client,
        ns.name(),
        &script_pod(
            "two",
            &[],
            "Never",
            &[
                Script {
                    name: "a",
                    script: "echo a; sleep 3600",
                },
                Script {
                    name: "b",
                    script: "echo b; sleep 3600",
                },
            ],
        ),
    )
    .await;
    wait_started(&client, ns.name(), "two").await;

    let kind_of = |result: oxikube_domain::OxiResult<oxikube_ports::LogStream>| {
        result.err().expect("an error").kind()
    };
    assert_eq!(
        kind_of(
            logs.stream_logs(ns.name(), "missing", &LogOptions::default())
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind_of(
            logs.stream_logs(ns.name(), "two", &LogOptions::default())
                .await
        ),
        ErrorKind::Validation,
        "two containers and no name"
    );
    assert_eq!(
        kind_of(
            logs.stream_logs(ns.name(), "two", &LogOptions::default().container("nope"))
                .await
        ),
        ErrorKind::NotFound
    );
}

#[tokio::test]
async fn a_multi_container_pod_carries_the_container_on_every_line() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    create(
        &client,
        ns.name(),
        &script_pod(
            "multi",
            &[],
            "Never",
            &[
                Script {
                    name: "alpha",
                    script: &burst("alpha", 30),
                },
                Script {
                    name: "beta",
                    script: &burst("beta", 20),
                },
            ],
        ),
    )
    .await;
    wait_started(&client, ns.name(), "multi").await;

    let stream = logs
        .stream_containers(
            ns.name(),
            "multi",
            &ContainerSelection::All,
            &LogOptions::default(),
        )
        .await
        .expect("fan-in");
    let lines: Vec<_> = stream.map(|l| l.expect("line")).collect().await;

    let of = |container: &str| -> Vec<_> {
        lines
            .iter()
            .filter(|l| &*l.container == container)
            .cloned()
            .collect()
    };
    assert_eq!(lines.len(), 50);
    assert!(lines.iter().all(|l| &*l.pod == "multi"));
    assert_eq!(numbers(&of("alpha"), "alpha"), (0..30).collect::<Vec<_>>());
    assert_eq!(numbers(&of("beta"), "beta"), (0..20).collect::<Vec<_>>());

    // Following, and naming one container.
    let named = ContainerSelection::Named(vec!["beta".into()]);
    let mut live = logs
        .stream_containers(ns.name(), "multi", &named, &LogOptions::follow())
        .await
        .expect("named");
    let lines = read_lines(&mut live, 20, WITHIN).await;
    assert!(lines.iter().all(|l| &*l.container == "beta"));
}

#[tokio::test]
async fn a_label_selector_fans_in_two_pods_and_picks_up_a_third() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    let labels = [("app", "fanin")];
    for name in ["web-1", "web-2"] {
        create(
            &client,
            ns.name(),
            &script_pod(
                name,
                &labels,
                "Never",
                &[Script {
                    name: "main",
                    script: &burst("hi", 10),
                }],
            ),
        )
        .await;
    }
    // A pod outside the selector must not show up.
    create(
        &client,
        ns.name(),
        &script_pod(
            "other",
            &[("app", "other")],
            "Never",
            &[Script {
                name: "main",
                script: &burst("no", 10),
            }],
        ),
    )
    .await;
    for name in ["web-1", "web-2", "other"] {
        wait_started(&client, ns.name(), name).await;
    }

    // Without follow: the pods present are read to the end and the stream finishes.
    let stream = logs
        .stream_selector(Some(ns.name()), "app=fanin", &LogOptions::default())
        .expect("selector");
    let lines: Vec<_> =
        tokio::time::timeout(WITHIN, stream.map(|l| l.expect("line")).collect::<Vec<_>>())
            .await
            .expect("the stream ends without follow");
    assert_eq!(lines.len(), 20);
    for pod in ["web-1", "web-2"] {
        let own: Vec<_> = lines.iter().filter(|l| &*l.pod == pod).cloned().collect();
        assert_eq!(numbers(&own, "hi"), (0..10).collect::<Vec<_>>(), "{pod}");
        assert!(own.iter().all(|l| &*l.container == "main"));
    }
    assert!(lines.iter().all(|l| &*l.pod != "other"));

    // With follow: a pod that starts matching later joins, read from its first line.
    let mut live = logs
        .stream_selector(Some(ns.name()), "app=fanin", &LogOptions::follow())
        .expect("follow selector");
    let first = read_lines(&mut live, 20, WITHIN).await;
    assert!(first.iter().all(|l| &*l.pod != "web-3"));
    create(
        &client,
        ns.name(),
        &script_pod(
            "web-3",
            &labels,
            "Never",
            &[Script {
                name: "main",
                script: &burst("late", 5),
            }],
        ),
    )
    .await;
    let late = read_lines(&mut live, 5, WITHIN).await;
    assert!(late.iter().all(|l| &*l.pod == "web-3"));
    assert_eq!(numbers(&late, "late"), (0..5).collect::<Vec<_>>());
}

/// Throughput of the adapter against a pod that logs as fast as busybox can: the budget is
/// 5 000 lines/s (docs/PERFORMANCE.md). Prints the numbers `--nocapture`.
#[tokio::test]
async fn a_busy_pod_streams_in_order_above_the_budget_rate() {
    const LINES: u32 = 100_000;
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let logs = logs(&client);
    create(
        &client,
        ns.name(),
        &script_pod(
            "busy",
            &[],
            "Never",
            &[Script {
                name: "main",
                script: &burst("n", LINES),
            }],
        ),
    )
    .await;
    wait_started(&client, ns.name(), "busy").await;
    // Let the pod finish writing so the read measures the adapter, not the writer.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut stream = logs
        .stream_logs(ns.name(), "busy", &LogOptions::default())
        .await
        .expect("stream");
    let started = Instant::now();
    let mut expected = 0u64;
    while let Some(line) = stream.next().await {
        let line = line.expect("line");
        assert_eq!(line.text, format!("n {expected}"), "in order, no loss");
        expected += 1;
    }
    let elapsed = started.elapsed();
    let rate = f64::from(LINES) / elapsed.as_secs_f64();
    eprintln!(
        "logs throughput: {LINES} lines in {elapsed:?} = {rate:.0} lines/s (batch {}, {} batches buffered)",
        logs.config().batch_size,
        logs.config().channel_batches,
    );
    assert_eq!(expected, u64::from(LINES));
    assert!(
        rate > 5_000.0,
        "{rate:.0} lines/s is below the 5 000 lines/s budget"
    );
}
