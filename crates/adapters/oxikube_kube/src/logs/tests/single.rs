//! `LogPort::stream_logs`: options, first-open errors, container resolution.

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{LogOptions, LogPort, LogSince};

use super::fake::{Chunk, FakeSource, collect, expect_lines, lines, pod, texts};
use crate::logs::source::ContainerState;

async fn error_of(source: &FakeSource, options: &LogOptions) -> OxiError {
    match source.logs().stream_logs("ns", "p", options).await {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

#[tokio::test(start_paused = true)]
async fn without_follow_the_log_is_read_once() {
    let source = FakeSource::new();
    source.reply("p", "app", lines(0..4));
    let opts = LogOptions::default().container("app").tail_lines(10);

    let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(texts(&items), expect_lines(0..4));
    let requests = source.requests("p");
    assert_eq!(requests.len(), 1, "no reconnect without follow");
    assert!(!requests[0].follow);
    assert_eq!(requests[0].tail_lines, Some(10));
}

#[tokio::test(start_paused = true)]
async fn a_read_that_breaks_without_follow_ends_with_an_error() {
    let source = FakeSource::new();
    source.reply("p", "app", [lines(0..2), vec![Chunk::Break]].concat());
    let opts = LogOptions::default().container("app");

    let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(items.len(), 3);
    assert_eq!(items[2].as_ref().unwrap_err().kind(), ErrorKind::Network);
    assert_eq!(source.requests("p").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn previous_is_a_single_read_even_when_follow_is_set() {
    let source = FakeSource::new();
    source.reply("p", "app", lines(0..3));
    let opts = LogOptions::follow().container("app").previous();

    let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(texts(&items), expect_lines(0..3));
    let requests = source.requests("p");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].previous && !requests[0].follow);
}

#[tokio::test(start_paused = true)]
async fn since_and_limit_bytes_reach_the_request_and_limit_disables_reconnects() {
    let source = FakeSource::new();
    source.reply("p", "app", lines(0..2));
    let opts = LogOptions::follow()
        .container("app")
        .since(LogSince::Seconds(60))
        .limit_bytes(4096);

    let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(items.len(), 2);
    let requests = source.requests("p");
    assert_eq!(
        requests.len(),
        1,
        "a byte limit ends the read; there is nothing to resume"
    );
    assert_eq!(requests[0].since, Some(LogSince::Seconds(60)));
    assert_eq!(requests[0].limit_bytes, Some(4096));
}

#[tokio::test(start_paused = true)]
async fn timestamps_are_stripped_whether_or_not_they_were_asked_for() {
    for timestamps in [false, true] {
        let source = FakeSource::new();
        source.reply("p", "app", lines(0..1));
        let mut opts = LogOptions::default().container("app");
        opts.timestamps = timestamps;

        let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

        let line = items[0].as_ref().unwrap();
        assert_eq!(line.text, "line 0");
        assert_eq!(line.ts, super::fake::ts(0));
    }
}

#[tokio::test(start_paused = true)]
async fn the_only_container_is_the_default() {
    let source = FakeSource::new();
    source.pod_state(pod("p", &[("web", ContainerState::Running, 0)]));
    source.reply("p", "web", lines(0..1));
    let opts = LogOptions::default();

    let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(&*items[0].as_ref().unwrap().container, "web");
}

#[tokio::test(start_paused = true)]
async fn the_default_container_annotation_is_honoured() {
    let source = FakeSource::new();
    let mut info = pod(
        "p",
        &[
            ("web", ContainerState::Running, 0),
            ("sidecar", ContainerState::Running, 0),
        ],
    );
    info.default_container = Some("sidecar".into());
    source.pod_state(info);
    source.reply("p", "sidecar", lines(0..1));

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &LogOptions::default())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(&*items[0].as_ref().unwrap().container, "sidecar");
}

#[tokio::test(start_paused = true)]
async fn several_containers_without_a_name_is_a_validation_error_listing_them() {
    let source = FakeSource::new();
    source.pod_state(pod(
        "p",
        &[
            ("web", ContainerState::Running, 0),
            ("sidecar", ContainerState::Running, 0),
        ],
    ));

    let err = error_of(&source, &LogOptions::default()).await;

    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("web") && err.message().contains("sidecar"));
    assert!(source.opens().is_empty(), "nothing was requested");
}

#[tokio::test(start_paused = true)]
async fn a_missing_pod_is_not_found() {
    let source = FakeSource::new();
    source.pod_gone("p");
    assert_eq!(
        error_of(&source, &LogOptions::default()).await.kind(),
        ErrorKind::NotFound
    );
}

#[tokio::test(start_paused = true)]
async fn first_open_errors_are_the_error_of_the_call() {
    for (error, kind) in [
        (
            OxiError::forbidden("pods/log is forbidden"),
            ErrorKind::Forbidden,
        ),
        (
            OxiError::validation("container is waiting to start"),
            ErrorKind::Validation,
        ),
        (OxiError::not_found("pod not found"), ErrorKind::NotFound),
        (OxiError::network("unreachable"), ErrorKind::Network),
    ] {
        let source = FakeSource::new();
        source.fail("p", "app", error);
        let opts = LogOptions::follow().container("app");
        assert_eq!(error_of(&source, &opts).await.kind(), kind);
        assert_eq!(source.requests("p").len(), 1, "no retry of the first open");
    }
}

#[tokio::test(start_paused = true)]
async fn an_unknown_container_name_is_not_found_even_though_the_server_says_400() {
    let source = FakeSource::new();
    source.pod_state(pod("p", &[("web", ContainerState::Running, 0)]));
    source.fail(
        "p",
        "nope",
        OxiError::validation("container nope is not valid for pod p"),
    );

    let err = error_of(&source, &LogOptions::default().container("nope")).await;

    assert_eq!(err.kind(), ErrorKind::NotFound);
}
