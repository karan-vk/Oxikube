//! The reconnect loop: overlap, dedup, restarts, end conditions, backoff and give-up.

use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{LogPort, LogSince};
use tokio::time::Instant;

use super::fake::{Chunk, FakeSource, collect, expect_lines, follow, lines, pod, texts, ts, wire};
use crate::logs::ContainerSelection;
use crate::logs::source::{ContainerState, PodPhase, RestartPolicy};

const APP: &str = "app";

fn running(restarts: i32) -> crate::logs::source::PodInfo {
    pod("p", &[(APP, ContainerState::Running, restarts)])
}

fn waiting(restarts: i32) -> crate::logs::source::PodInfo {
    pod("p", &[(APP, ContainerState::Waiting, restarts)])
}

#[tokio::test(start_paused = true)]
async fn reconnect_resumes_from_the_overlap_and_drops_the_replay() {
    let source = FakeSource::new();
    let mut first = lines(0..5);
    first.push(Chunk::Break);
    source.reply("p", APP, first);
    // The overlap replays 3 and 4; 5..8 are new.
    source.reply("p", APP, lines(3..8));
    source
        .pod_state(running(0))
        .pod_state(running(0))
        .pod_gone("p");

    let stream = source
        .logs()
        .stream_logs("ns", "p", &follow())
        .await
        .unwrap();
    let items = collect(stream).await;

    assert_eq!(texts(&items), expect_lines(0..8), "no duplicates, no gap");
    let requests = source.requests("p");
    assert_eq!(requests.len(), 2);
    assert!(requests[0].follow && requests[0].since.is_none());
    assert_eq!(
        requests[1].since,
        Some(LogSince::Time(
            ts(4)
                .checked_sub(jiff::SignedDuration::from_secs(5))
                .unwrap()
        )),
        "resume five seconds before the last line seen"
    );
    assert!(requests[1].follow && requests[1].tail_lines.is_none() && !requests[1].previous);
}

#[tokio::test(start_paused = true)]
async fn the_first_open_keeps_tail_and_the_reconnect_drops_it() {
    let source = FakeSource::new();
    source.reply("p", APP, [lines(0..2), vec![Chunk::Break]].concat());
    source.reply("p", APP, lines(1..3));
    source
        .pod_state(running(0))
        .pod_state(running(0))
        .pod_gone("p");

    let opts = follow().tail_lines(100);
    let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(texts(&items), expect_lines(0..3));
    let requests = source.requests("p");
    assert_eq!(requests[0].tail_lines, Some(100));
    assert_eq!(requests[1].tail_lines, None);
}

#[tokio::test(start_paused = true)]
async fn a_reconnect_does_not_replay_lines_older_than_the_tail_window() {
    let source = FakeSource::new();
    source.reply("p", APP, [lines(7..10), vec![Chunk::Break]].concat());
    // The overlap reaches back to line 0, well before the three lines the tail asked for.
    source.reply("p", APP, lines(0..12));
    source
        .pod_state(running(0))
        .pod_state(running(0))
        .pod_gone("p");

    let opts = follow().tail_lines(3);
    let items = collect(source.logs().stream_logs("ns", "p", &opts).await.unwrap()).await;

    assert_eq!(
        texts(&items),
        expect_lines(7..12),
        "in order, nothing from before the window"
    );
}

#[tokio::test(start_paused = true)]
async fn repeated_text_at_different_times_survives_a_replay() {
    let source = FakeSource::new();
    let tick = |n| Chunk::Bytes(wire(n, "tick"));
    source.reply("p", APP, vec![tick(0), tick(1), tick(2), Chunk::Break]);
    // Replay of the three, plus a fourth genuine "tick" a moment later.
    source.reply("p", APP, vec![tick(0), tick(1), tick(2), tick(3)]);
    source
        .pod_state(running(0))
        .pod_state(running(0))
        .pod_gone("p");

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(texts(&items), ["tick"; 4]);
}

#[tokio::test(start_paused = true)]
async fn lines_carry_the_pod_container_and_timestamp() {
    let source = FakeSource::new();
    source.reply("p", APP, lines(0..1));
    source.pod_state(running(0));
    // After the stream ends the container is done and will not restart.
    let mut finished = pod(
        "p",
        &[(APP, ContainerState::Terminated { exit_code: 0 }, 0)],
    );
    finished.restart_policy = RestartPolicy::Never;
    source.pod_state(finished);

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;
    let line = items[0].as_ref().unwrap();
    assert_eq!((&*line.pod, &*line.container), ("p", APP));
    assert_eq!(line.ts, ts(0));
    assert_eq!(line.text, "line 0");
}

#[tokio::test(start_paused = true)]
async fn a_finished_container_ends_the_stream_after_one_open() {
    let source = FakeSource::new();
    source.reply("p", APP, lines(0..3));
    let mut done = pod(
        "p",
        &[(APP, ContainerState::Terminated { exit_code: 0 }, 0)],
    );
    done.restart_policy = RestartPolicy::Never;
    done.phase = PodPhase::Succeeded;
    source.pod_state(done);

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(texts(&items), expect_lines(0..3));
    assert_eq!(
        source.requests("p").len(),
        1,
        "no reconnect for a finished container"
    );
}

#[tokio::test(start_paused = true)]
async fn a_deleted_pod_ends_the_stream_without_an_error() {
    let source = FakeSource::new();
    source.reply("p", APP, [lines(0..2), vec![Chunk::Break]].concat());
    source.pod_state(running(0)).pod_gone("p");

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(texts(&items), expect_lines(0..2));
    assert_eq!(source.requests("p").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_pod_recreated_under_the_same_name_ends_the_stream() {
    let source = FakeSource::new();
    source.reply("p", APP, [lines(0..2), vec![Chunk::Break]].concat());
    let mut recreated = running(0);
    recreated.uid = "another-uid".into();
    source.pod_state(running(0)).pod_state(recreated);

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(texts(&items), expect_lines(0..2));
    assert_eq!(
        source.requests("p").len(),
        1,
        "the new pod's log is not ours"
    );
}

#[tokio::test(start_paused = true)]
async fn a_restart_reads_the_previous_instance_to_close_the_gap() {
    let source = FakeSource::new();
    // Lines 0..3 arrive, then the connection drops; line 3 and 4 are written by the old
    // instance before it dies, the new instance starts at 5.
    source.reply("p", APP, [lines(0..3), vec![Chunk::Break]].concat());
    source.reply("p", APP, lines(2..5)); // previous instance: 2 is the overlap
    source.reply("p", APP, lines(5..8)); // current instance
    source
        .pod_state(running(0))
        .pod_state(waiting(1))
        .pod_gone("p");

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(
        texts(&items),
        expect_lines(0..8),
        "no gap across the restart"
    );
    let requests = source.requests("p");
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1].previous && !requests[1].follow,
        "previous instance first"
    );
    assert!(
        requests[2].follow && !requests[2].previous,
        "then the current one"
    );
}

#[tokio::test(start_paused = true)]
async fn waiting_to_restart_is_not_counted_as_a_failure() {
    let source = FakeSource::new();
    source.reply("p", APP, [lines(0..2), vec![Chunk::Break]].concat());
    // The previous-instance read: nothing to read.
    source.fail(
        "p",
        APP,
        OxiError::not_found("previous terminated container not found"),
    );
    // CrashLoopBackOff: more waiting opens than `max_open_failures`.
    for _ in 0..12 {
        source.fail(
            "p",
            APP,
            OxiError::validation("container is waiting to start"),
        );
    }
    source.reply("p", APP, lines(2..4));
    source.pod_state(running(0)).pod_state(waiting(1));
    // After the second response the pod is gone.
    let started = Instant::now();
    let stream = source
        .logs()
        .stream_logs("ns", "p", &follow())
        .await
        .unwrap();
    // `waiting(1)` repeats for every probe, so end the stream by consuming four lines.
    let items: Vec<_> = stream.take(4).collect().await;

    assert_eq!(texts(&items), expect_lines(0..4));
    assert!(
        started.elapsed() >= Duration::from_secs(10),
        "backed off between attempts"
    );
}

#[tokio::test(start_paused = true)]
async fn a_container_without_a_status_yet_is_waited_for() {
    let source = FakeSource::new();
    source.pod_state(pod("p", &[(APP, ContainerState::Unknown, 0)]));
    // A pod that is not scheduled for longer than `max_open_failures` backoffs.
    for _ in 0..12 {
        source.fail(
            "p",
            APP,
            OxiError::validation("container is waiting to start"),
        );
    }
    source.reply("p", APP, [lines(0..2), vec![Chunk::Hang]].concat());

    let stream = source
        .logs()
        .stream_containers("ns", "p", &ContainerSelection::All, &follow())
        .await
        .unwrap();
    let items: Vec<_> = stream.take(2).collect().await;

    assert_eq!(texts(&items), expect_lines(0..2));
    assert_eq!(source.requests("p").len(), 13);
}

#[tokio::test(start_paused = true)]
async fn a_container_without_a_status_in_a_finished_pod_is_given_up_on() {
    let source = FakeSource::new();
    let mut info = pod("p", &[(APP, ContainerState::Unknown, 0)]);
    info.phase = PodPhase::Failed;
    source.pod_state(info);
    for _ in 0..20 {
        source.fail(
            "p",
            APP,
            OxiError::validation("container is waiting to start"),
        );
    }

    let stream = source
        .logs()
        .stream_containers("ns", "p", &ContainerSelection::All, &follow())
        .await
        .unwrap();
    let items = collect(stream).await;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].as_ref().unwrap_err().kind(), ErrorKind::Validation);
    assert_eq!(source.requests("p").len(), 10);
}

#[tokio::test(start_paused = true)]
async fn gives_up_after_max_open_failures_with_the_last_error() {
    let source = FakeSource::new();
    source.reply("p", APP, [lines(0..1), vec![Chunk::Break]].concat());
    for _ in 0..20 {
        source.fail("p", APP, OxiError::network("connection refused"));
    }
    source.pod_state(running(0));

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].as_ref().unwrap().text, "line 0");
    assert_eq!(items[1].as_ref().unwrap_err().kind(), ErrorKind::Network);
    assert_eq!(
        source.requests("p").len(),
        1 + 10,
        "one open plus ten failed reopens"
    );
}

#[tokio::test(start_paused = true)]
async fn a_final_error_on_reconnect_ends_the_stream() {
    let source = FakeSource::new();
    source.reply("p", APP, [lines(0..1), vec![Chunk::Break]].concat());
    source.fail("p", APP, OxiError::forbidden("pods/log is forbidden"));
    source.pod_state(running(0));

    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(items.len(), 2);
    assert_eq!(items[1].as_ref().unwrap_err().kind(), ErrorKind::Forbidden);
    assert_eq!(source.requests("p").len(), 2, "forbidden is not retried");
}

#[tokio::test(start_paused = true)]
async fn backoff_grows_while_nothing_arrives() {
    let source = FakeSource::new();
    // Five streams that end immediately with no lines, then one with a line.
    for _ in 0..5 {
        source.reply("p", APP, vec![]);
    }
    source.reply("p", APP, lines(0..1));
    let mut finished = pod(
        "p",
        &[(APP, ContainerState::Terminated { exit_code: 0 }, 0)],
    );
    finished.restart_policy = RestartPolicy::Never;
    // Running for the first probes (container running, stream just ended), then finished.
    for _ in 0..6 {
        source.pod_state(running(0));
    }
    source.pod_state(finished);

    let started = Instant::now();
    let items = collect(
        source
            .logs()
            .stream_logs("ns", "p", &follow())
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(texts(&items), expect_lines(0..1));
    // 250 + 500 + 1000 + 2000 + 4000 ms between the six opens.
    assert!(started.elapsed() >= Duration::from_millis(7750));
    assert!(started.elapsed() < Duration::from_secs(12));
}
