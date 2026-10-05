//! Multi-container and label-selector fan-in.

use futures::StreamExt;
use oxikube_domain::log::LogLine;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{LogOptions, LogSince};

use super::fake::{Chunk, FakeSource, collect, lines, pod, wire};
use crate::logs::source::{ContainerState, PodEvent, PodInfo};
use crate::logs::{ContainerSelection, LogsConfig};

fn two_containers() -> PodInfo {
    pod(
        "p",
        &[
            ("app", ContainerState::Running, 0),
            ("sidecar", ContainerState::Running, 0),
        ],
    )
}

/// `(pod, container)` and text of every line.
fn sources(items: &[oxikube_domain::OxiResult<LogLine>]) -> Vec<(String, String, String)> {
    items
        .iter()
        .filter_map(|i| i.as_ref().ok())
        .map(|l| (l.pod.to_string(), l.container.to_string(), l.text.clone()))
        .collect()
}

fn of<'a>(all: &'a [(String, String, String)], pod: &str, container: &str) -> Vec<&'a str> {
    all.iter()
        .filter(|(p, c, _)| p == pod && c == container)
        .map(|(_, _, t)| t.as_str())
        .collect()
}

fn init_pod(name: &str, container: &str, state: ContainerState, init: bool) -> PodInfo {
    let mut info = pod(name, &[(container, state, 0)]);
    info.containers[0].init = init;
    info
}

#[tokio::test(start_paused = true)]
async fn every_container_of_a_pod_is_merged_with_its_source() {
    let source = FakeSource::new();
    let mut info = two_containers();
    info.containers.insert(
        0,
        init_pod(
            "p",
            "init",
            ContainerState::Terminated { exit_code: 0 },
            true,
        )
        .containers
        .remove(0),
    );
    source.pod_state(info);
    source.reply("p", "init", lines(0..2));
    source.reply("p", "app", lines(0..3));
    source.reply("p", "sidecar", lines(0..4));

    let stream = source
        .logs()
        .stream_containers("ns", "p", &ContainerSelection::All, &LogOptions::default())
        .await
        .unwrap();
    let all = sources(&collect(stream).await);

    assert_eq!(all.len(), 9);
    assert_eq!(of(&all, "p", "init"), ["line 0", "line 1"]);
    assert_eq!(of(&all, "p", "app"), ["line 0", "line 1", "line 2"]);
    assert_eq!(of(&all, "p", "sidecar").len(), 4);
}

#[tokio::test(start_paused = true)]
async fn named_containers_only_and_unknown_names_are_rejected() {
    let source = FakeSource::new();
    source.pod_state(two_containers());
    source.reply("p", "sidecar", lines(0..2));

    let named = ContainerSelection::Named(vec!["sidecar".into()]);
    let stream = source
        .logs()
        .stream_containers("ns", "p", &named, &LogOptions::default())
        .await
        .unwrap();
    assert_eq!(sources(&collect(stream).await).len(), 2);
    assert!(
        source
            .requests("p")
            .iter()
            .all(|r| r.container == "sidecar")
    );

    let unknown = ContainerSelection::Named(vec!["nope".into()]);
    let err = source
        .logs()
        .stream_containers("ns", "p", &unknown, &LogOptions::default())
        .await
        .err()
        .expect("error");
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[tokio::test(start_paused = true)]
async fn a_missing_pod_is_not_found() {
    let source = FakeSource::new();
    source.pod_gone("p");
    let err = source
        .logs()
        .stream_containers("ns", "p", &ContainerSelection::All, &LogOptions::default())
        .await
        .err()
        .expect("error");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test(start_paused = true)]
async fn without_follow_containers_that_never_started_are_skipped() {
    let source = FakeSource::new();
    source.pod_state(pod(
        "p",
        &[
            ("app", ContainerState::Running, 0),
            ("late", ContainerState::Waiting, 0),
        ],
    ));
    source.reply("p", "app", lines(0..2));

    let stream = source
        .logs()
        .stream_containers("ns", "p", &ContainerSelection::All, &LogOptions::default())
        .await
        .unwrap();
    let items = collect(stream).await;

    assert_eq!(sources(&items).len(), 2);
    assert!(items.iter().all(Result::is_ok));
    assert!(source.requests("p").iter().all(|r| r.container == "app"));
}

#[tokio::test(start_paused = true)]
async fn a_container_that_cannot_be_read_does_not_end_the_others() {
    let source = FakeSource::new();
    source.pod_state(two_containers());
    source.reply("p", "app", lines(0..3));
    source.fail("p", "sidecar", OxiError::forbidden("pods/log is forbidden"));

    let stream = source
        .logs()
        .stream_containers("ns", "p", &ContainerSelection::All, &LogOptions::default())
        .await
        .unwrap();
    let items = collect(stream).await;

    assert_eq!(sources(&items).len(), 3);
    let errors: Vec<_> = items.iter().filter_map(|i| i.as_ref().err()).collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind(), ErrorKind::Forbidden);
}

fn added(pod: PodInfo, initial: bool) -> oxikube_domain::OxiResult<PodEvent> {
    Ok(PodEvent::Pod { pod, initial })
}

fn named_pod(name: &str, container: &str, state: ContainerState) -> PodInfo {
    pod(name, &[(container, state, 0)])
}

#[tokio::test(start_paused = true)]
async fn a_selector_without_follow_reads_the_pods_present_and_ends() {
    let source = FakeSource::new();
    let events = source.watch();
    source.reply("a", "app", lines(0..2));
    source.reply("b", "app", lines(0..3));
    events
        .send(added(named_pod("a", "app", ContainerState::Running), true))
        .unwrap();
    events
        .send(added(named_pod("b", "app", ContainerState::Running), true))
        .unwrap();
    events.send(Ok(PodEvent::InitDone)).unwrap();

    let stream = source
        .logs()
        .stream_selector(Some("ns"), "app=web", &LogOptions::default())
        .unwrap();
    let all = sources(&collect(stream).await);

    assert_eq!(of(&all, "a", "app").len(), 2);
    assert_eq!(of(&all, "b", "app").len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_followed_selector_picks_up_pods_that_appear_later() {
    let source = FakeSource::new();
    let events = source.watch();
    source.reply("a", "app", [lines(0..1), vec![Chunk::Hang]].concat());
    source.reply(
        "b",
        "app",
        [vec![Chunk::Bytes(wire(5, "from b"))], vec![Chunk::Hang]].concat(),
    );
    events
        .send(added(named_pod("a", "app", ContainerState::Running), true))
        .unwrap();
    events.send(Ok(PodEvent::InitDone)).unwrap();

    let options = LogOptions::follow()
        .tail_lines(5)
        .since(LogSince::Seconds(60));
    let mut stream = source
        .logs()
        .stream_selector(Some("ns"), "app=web", &options)
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().text, "line 0");

    // A new pod starts matching: it is read from its first line.
    events
        .send(added(named_pod("b", "app", ContainerState::Running), false))
        .unwrap();
    let line = stream.next().await.unwrap().unwrap();
    assert_eq!((&*line.pod, line.text.as_str()), ("b", "from b"));

    let a = &source.requests("a")[0];
    assert_eq!(
        (a.tail_lines, a.since),
        (Some(5), Some(LogSince::Seconds(60)))
    );
    let b = &source.requests("b")[0];
    assert_eq!(
        (b.tail_lines, b.since),
        (None, None),
        "a joiner is read from the start"
    );
}

#[tokio::test(start_paused = true)]
async fn a_pending_container_is_followed_once_it_starts() {
    let source = FakeSource::new();
    let events = source.watch();
    source.reply("b", "app", [lines(0..1), vec![Chunk::Hang]].concat());
    events
        .send(added(named_pod("b", "app", ContainerState::Waiting), true))
        .unwrap();
    events.send(Ok(PodEvent::InitDone)).unwrap();

    let mut stream = source
        .logs()
        .stream_selector(None, "app=web", &LogOptions::follow())
        .unwrap();
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(
        source.requests("b").is_empty(),
        "nothing to read before it starts"
    );

    events
        .send(added(named_pod("b", "app", ContainerState::Running), false))
        .unwrap();
    // And a repeated update of a followed container does not start a second reader.
    events
        .send(added(named_pod("b", "app", ContainerState::Running), false))
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().text, "line 0");
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(source.requests("b").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_pod_recreated_under_the_same_name_is_followed_again() {
    let source = FakeSource::new();
    let events = source.watch();
    source.reply(
        "a",
        "app",
        [vec![Chunk::Bytes(wire(0, "old"))], vec![Chunk::Hang]].concat(),
    );
    source.reply(
        "a",
        "app",
        [vec![Chunk::Bytes(wire(9, "new"))], vec![Chunk::Hang]].concat(),
    );
    events
        .send(added(named_pod("a", "app", ContainerState::Running), true))
        .unwrap();
    events.send(Ok(PodEvent::InitDone)).unwrap();

    let mut stream = source
        .logs()
        .stream_selector(Some("ns"), "app=web", &LogOptions::follow())
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().text, "old");

    let mut recreated = named_pod("a", "app", ContainerState::Running);
    recreated.uid = "uid-2".into();
    events.send(added(recreated, false)).unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().text, "new");
}

#[tokio::test(start_paused = true)]
async fn the_container_limit_caps_the_followers() {
    let source = FakeSource::new();
    source.pod_state(two_containers());
    source.reply("p", "app", lines(0..1));
    source.reply("p", "sidecar", lines(0..1));
    let logs = source.logs_with(LogsConfig {
        max_fanin_streams: 1,
        ..LogsConfig::default()
    });

    let stream = logs
        .stream_containers("ns", "p", &ContainerSelection::All, &LogOptions::default())
        .await
        .unwrap();

    assert_eq!(sources(&collect(stream).await).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_watch_that_cannot_work_ends_the_stream_with_its_error() {
    let source = FakeSource::new();
    let events = source.watch();
    events
        .send(Err(OxiError::forbidden("pods is forbidden")))
        .unwrap();

    let stream = source
        .logs()
        .stream_selector(Some("ns"), "app=web", &LogOptions::follow())
        .unwrap();
    let items = collect(stream).await;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].as_ref().unwrap_err().kind(), ErrorKind::Forbidden);
}

#[test]
fn an_empty_selector_is_rejected() {
    let source = FakeSource::new();
    let err = source
        .logs()
        .stream_selector(None, "  ", &LogOptions::follow())
        .err()
        .expect("error");
    assert_eq!(err.kind(), ErrorKind::Validation);
}
