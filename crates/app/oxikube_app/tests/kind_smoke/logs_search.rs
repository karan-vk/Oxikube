//! The match index (E08-S03) over a live `LogService` session: a busybox pod that echoes an
//! `ERROR` line every fifth line and an `INFO` line otherwise, 20 a second, read into a 100-line
//! ring while a `MatchIndex` follows it the way the viewer's deltas do.
//!
//! * the index, brought up to date while lines stream in and the ring drops the oldest, equals a
//!   naive scan of what the ring holds;
//! * an inverse index is the complement; a case-sensitive pattern differs from the insensitive one;
//! * next / previous wrap and never land on a dropped line.
//!
//! The pod lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::logs::{
    LogConfig, LogFilter, LogRuntime, LogService, LogTarget, MIN_BUFFER_LINES, MatchIndex,
};
use oxikube_app::store::Spawner;
use oxikube_kube::KubeLogs;
use oxikube_ports::LogOptions;
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::TestNamespace;

use crate::clock::TokioClock;
use crate::cluster::Kind;
use crate::eventually;

fn chatty_pod(name: &str) -> Pod {
    serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { "oxikube.test/suite": "logs-search" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [{
                "name": "main",
                "image": BUSYBOX,
                "command": ["sh", "-c",
                    "i=0; while true; do \
                       if [ $((i % 5)) = 4 ]; then echo \"ERROR line $i\"; else echo \"INFO line $i\"; fi; \
                       i=$((i+1)); sleep 0.05; done"],
            }],
        },
    }))
    .expect("a pod")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_match_index_follows_a_live_stream_and_equals_a_full_scan() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    pods.create(&PostParams::default(), &chatty_pod("chatty"))
        .await
        .expect("create the pod");
    eventually("the pod to run", String::new, || async {
        pods.get("chatty")
            .await
            .ok()
            .and_then(|p| p.status)
            .and_then(|s| s.phase)
            .is_some_and(|phase| phase == "Running")
    })
    .await;

    let spawner: Arc<dyn Spawner> = Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    });
    let service = LogService::new(
        LogRuntime {
            spawner,
            clock: Arc::new(TokioClock),
        },
        LogConfig {
            buffer_lines: MIN_BUFFER_LINES,
            ..LogConfig::default()
        },
    );
    let session = service.open(
        Arc::new(KubeLogs::new(client)),
        LogTarget::pod(ns.name(), "chatty"),
        LogOptions::follow(),
    );
    let matcher = |filter: LogFilter| Arc::new(filter.compile().expect("a valid pattern"));
    let mut errors = MatchIndex::new(matcher(LogFilter::new("error")));
    let mut sensitive = MatchIndex::new(matcher(LogFilter {
        pattern: "error".to_owned(),
        case_sensitive: true,
        inverse: false,
    }));
    let mut not_errors = MatchIndex::new(matcher(LogFilter {
        pattern: "error".to_owned(),
        case_sensitive: false,
        inverse: true,
    }));

    // Follow the stream for as long as it takes the 100-line ring to drop lines, bringing the
    // indexes up to date between reads like the viewer does with each delta.
    let reader = session.reader();
    eventually(
        "the ring to drop lines while the indexes follow",
        || format!("{:?} with {} lines", reader.state(), reader.len()),
        || {
            let done = reader.read(|buffer, _| {
                errors.catch_up(buffer);
                sensitive.catch_up(buffer);
                not_errors.catch_up(buffer);
                buffer.dropped() >= 20
            });
            async move { done }
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    reader.read(|buffer, _| {
        errors.catch_up(buffer);
        sensitive.catch_up(buffer);
        not_errors.catch_up(buffer);
        let naive = |wanted: &dyn Fn(&str) -> bool| -> Vec<u64> {
            buffer
                .iter()
                .filter(|e| wanted(&e.text))
                .map(|e| e.seq)
                .collect()
        };
        let want_errors = naive(&|t| t.starts_with("ERROR"));
        assert!(!want_errors.is_empty());
        assert_eq!(errors.iter().collect::<Vec<_>>(), want_errors);
        assert_eq!(
            sensitive.iter().collect::<Vec<_>>(),
            Vec::<u64>::new(),
            "`error` in lower case matches nothing: the pod writes `ERROR`"
        );
        assert_eq!(
            not_errors.iter().collect::<Vec<_>>(),
            naive(&|t| t.starts_with("INFO"))
        );
        assert_eq!(errors.len() + not_errors.len(), buffer.len());
        // Every kept match is still retained, and the lines the ring dropped are gone.
        assert!(errors.iter().all(|seq| buffer.get_seq(seq).is_some()));
        assert!(errors.first().unwrap() >= buffer.first_seq());
        // Next and previous wrap, and a current match the ring dropped is skipped.
        let first = errors.first().unwrap();
        let last = errors.last().unwrap();
        assert_eq!(errors.next(Some(last), 0), Some(first));
        assert_eq!(errors.prev(Some(first)), Some(last));
        let dropped = buffer.first_seq().saturating_sub(1);
        assert_eq!(errors.next(Some(dropped), 0), Some(first));
    });
}
