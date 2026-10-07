//! `read_excerpt` over `FakeLogPort` / `FakeResourcePort` scripts, on the deterministic executor
//! and the fake clock (no runtime, no threads).

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::LogSince;
use oxikube_testkit::{LogCall, Timeline, deployment};

use super::harness::{Env, line, ms, ts, web_pod};
use super::*;
use crate::logs::{AggregateSpec, LogFilter};

fn pod_request() -> ExcerptRequest {
    ExcerptRequest::new(ExcerptSource::Pod {
        namespace: "default".into(),
        pod: "web-0".into(),
        container: None,
    })
}

fn read(env: &mut Env, request: &ExcerptRequest) -> oxikube_domain::OxiResult<LogExcerpt> {
    let (service, ports) = (env.service.clone(), env.ports());
    env.run(async move { service.read_excerpt(ports, request).await })
}

#[test]
fn a_pod_read_returns_the_newest_lines_with_server_time_and_pod() {
    let mut env = Env::new();
    env.script(env.lines(5));
    let excerpt = read(&mut env, &pod_request().tail(3)).unwrap();
    assert_eq!(
        excerpt.text,
        "2025-10-09T08:53:20.002Z web-0/app line 2\n\
         2025-10-09T08:53:20.003Z web-0/app line 3\n\
         2025-10-09T08:53:20.004Z web-0/app line 4\n"
    );
    assert_eq!(excerpt.lines, 3);
    assert_eq!(
        (excerpt.scanned, excerpt.matched, excerpt.omitted),
        (5, 5, 2)
    );
    assert_eq!(excerpt.span, Some((ts(2), ts(4))));
    assert_eq!(excerpt.notes().len(), 1, "the two older lines are noted");

    // A read that fits says nothing more.
    env.script(env.lines(3));
    let whole = read(&mut env, &pod_request()).unwrap();
    assert_eq!(whole.lines, 3);
    assert!(whole.notes().is_empty());
}

#[test]
fn the_read_never_follows_and_passes_since_and_tail_to_the_port() {
    let mut env = Env::new();
    env.script(env.lines(2));
    let request = pod_request().tail(50).since(parse_since("10m").unwrap());
    read(&mut env, &request).unwrap();
    let calls = env.logs.recorded_calls();
    let [
        LogCall::StreamLogs {
            namespace,
            pod,
            options,
        },
    ] = &calls[..]
    else {
        panic!("one stream: {calls:?}")
    };
    assert_eq!((namespace.as_str(), pod.as_str()), ("default", "web-0"));
    assert!(!options.follow, "a tool call never follows");
    assert!(options.timestamps);
    assert_eq!(options.since, Some(LogSince::Seconds(600)));
    assert_eq!(options.tail_lines, Some(50));
}

#[test]
fn a_container_is_passed_through() {
    let mut env = Env::new();
    env.script(env.lines(1));
    let request = ExcerptRequest::new(ExcerptSource::Pod {
        namespace: "default".into(),
        pod: "web-0".into(),
        container: Some("sidecar".into()),
    });
    read(&mut env, &request).unwrap();
    let calls = env.logs.recorded_calls();
    let [LogCall::StreamLogs { options, .. }] = &calls[..] else {
        panic!()
    };
    assert_eq!(options.container.as_deref(), Some("sidecar"));
}

#[test]
fn grep_keeps_the_matching_lines_and_widens_the_read() {
    let mut env = Env::new();
    env.script(Timeline::immediate([
        line("web-0", 0, "ok one"),
        line("web-0", 1, "ERROR boom"),
        line("web-0", 2, "ok two"),
        line("web-0", 3, "error again"),
    ]));
    let request = pod_request().tail(10).matching(LogFilter::new("error"));
    let excerpt = read(&mut env, &request).unwrap();
    let texts: Vec<_> = excerpt
        .text
        .lines()
        .map(|l| l.rsplit_once(' ').unwrap().0.len())
        .collect();
    assert_eq!(texts.len(), 2);
    assert!(excerpt.text.contains("ERROR boom") && excerpt.text.contains("error again"));
    assert_eq!((excerpt.scanned, excerpt.matched, excerpt.lines), (4, 2, 2));
    let calls = env.logs.recorded_calls();
    let [LogCall::StreamLogs { options, .. }] = &calls[..] else {
        panic!()
    };
    assert_eq!(
        options.tail_lines,
        Some(SCAN_LINES as i64),
        "a filter searches more than the tail"
    );
}

#[test]
fn an_inverse_grep_returns_the_lines_without_the_pattern() {
    let mut env = Env::new();
    env.script(Timeline::immediate([
        line("web-0", 0, "health ok"),
        line("web-0", 1, "real work"),
    ]));
    let filter = LogFilter {
        inverse: true,
        ..LogFilter::new("health")
    };
    let excerpt = read(&mut env, &pod_request().matching(filter)).unwrap();
    assert_eq!(excerpt.lines, 1);
    assert!(excerpt.text.contains("real work"));
}

#[test]
fn an_invalid_pattern_is_a_validation_error() {
    let mut env = Env::new();
    env.script(env.lines(1));
    let err = read(
        &mut env,
        &pod_request().matching(LogFilter::new("(unclosed")),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().starts_with("grep: "), "{}", err.message());
    assert!(
        env.logs.recorded_calls().is_empty(),
        "nothing is read for a bad pattern"
    );
}

#[test]
fn lines_beyond_the_tail_are_counted_and_noted() {
    let mut env = Env::new();
    env.script(env.lines(30));
    // The fake port ignores tail_lines, so all 30 reach the buffer; the excerpt keeps the newest 5.
    let excerpt = read(&mut env, &pod_request().tail(5)).unwrap();
    assert_eq!(
        (excerpt.lines, excerpt.matched, excerpt.omitted),
        (5, 30, 25)
    );
    assert!(excerpt.text.ends_with("line 29\n"));
    let notes = excerpt.notes().join(" ");
    assert!(
        notes.contains("25 older matching lines were omitted beyond the requested tail"),
        "{notes}"
    );
}

#[test]
fn the_byte_budget_cuts_the_oldest_lines_and_says_so() {
    let mut env = Env::new();
    env.script(Timeline::immediate(
        (0..100).map(|i| line("web-0", i, &format!("{i:0>200}"))),
    ));
    let excerpt = read(&mut env, &pod_request().tail(100).max_bytes(2_000)).unwrap();
    assert!(excerpt.text.len() <= 2_000, "{}", excerpt.text.len());
    assert!(excerpt.budget_cut);
    assert!(excerpt.lines < 100 && excerpt.lines > 0);
    assert!(
        excerpt.text.ends_with(&format!("{:0>200}\n", 99)),
        "the newest line is kept"
    );
    assert!(excerpt.notes().join(" ").contains("size limit"));
}

#[test]
fn a_bearer_token_is_masked_in_the_output() {
    let mut env = Env::new();
    env.script(Timeline::immediate([
        line(
            "web-0",
            0,
            "calling api with Authorization: Bearer sk-live-abcdefghijklmnopqrstuvwxyz",
        ),
        line("web-0", 1, "password=hunter2hunter2 retry"),
        line("web-0", 2, "plain line"),
    ]));
    let excerpt = read(&mut env, &pod_request()).unwrap();
    for secret in ["sk-live-abcdefghijklmnopqrstuvwxyz", "hunter2hunter2"] {
        assert!(!excerpt.text.contains(secret), "{}", excerpt.text);
    }
    assert!(excerpt.text.contains("[redacted]"));
    assert!(excerpt.text.contains("plain line"));
}

#[test]
fn a_missing_pod_is_the_ports_error() {
    let mut env = Env::new();
    env.logs
        .script()
        .stream_logs
        .push_err(OxiError::not_found("pods \"web-0\" not found"));
    let err = read(&mut env, &pod_request()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("web-0"));
}

#[test]
fn a_stream_that_never_ends_is_cut_at_the_deadline_and_closed() {
    let mut env = Env::new();
    env.script(
        Timeline::new()
            .ok_at(ms(0), line("web-0", 0, "first"))
            .keep_open(),
    );
    let excerpt = read(&mut env, &pod_request()).unwrap();
    assert!(excerpt.timed_out);
    assert!(excerpt.text.contains("first"));
    assert!(excerpt.notes().join(" ").contains("time limit"));
    env.settle();
    assert_eq!(
        env.logs.live_streams(),
        0,
        "the stream is closed with the read"
    );
}

#[test]
fn a_workload_read_merges_its_pods_by_server_timestamp() {
    let mut env = Env::new();
    env.resources
        .insert(deployment().name("web").namespace("default").build());
    env.resources.insert(web_pod("web-a"));
    env.resources.insert(web_pod("web-b"));
    env.script(Timeline::immediate([
        line("web-a", 10, "a10"),
        line("web-a", 40, "a40"),
    ]));
    env.script(Timeline::immediate([
        line("web-b", 20, "b20"),
        line("web-b", 50, "b50"),
    ]));
    let spec = AggregateSpec {
        namespace: "default".into(),
        source: crate::logs::AggregateSource::Object {
            gvk: oxikube_domain::ids::Gvk::new("apps", "v1", "Deployment"),
            name: "web".into(),
        },
        extra_selector: None,
        container: None,
    };
    let request = ExcerptRequest::new(ExcerptSource::Workload(spec));
    let excerpt = read(&mut env, &request).unwrap();
    let order: Vec<_> = excerpt
        .text
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap().to_owned())
        .collect();
    assert_eq!(order, ["a10", "b20", "a40", "b50"]);
    assert!(excerpt.text.contains("web-a/app") && excerpt.text.contains("web-b/app"));
    assert_eq!(excerpt.streams, 2);
    assert_eq!(excerpt.matched_pods, Some(2));
    for call in env.logs.recorded_calls() {
        let LogCall::StreamLogs { options, .. } = call;
        assert!(!options.follow);
    }
}

#[test]
fn a_selector_read_with_no_pods_is_empty_not_an_error() {
    let mut env = Env::new();
    let request = ExcerptRequest::new(ExcerptSource::Workload(AggregateSpec::selector(
        "default",
        "app=nothing",
    )));
    let excerpt = read(&mut env, &request).unwrap();
    assert!(excerpt.is_empty());
    assert_eq!(excerpt.matched_pods, Some(0));
}

#[test]
fn a_pod_that_cannot_be_read_is_noted_and_the_others_are_returned() {
    let mut env = Env::new();
    env.resources.insert(web_pod("web-a"));
    env.resources.insert(web_pod("web-b"));
    env.script(Timeline::immediate([line("web-a", 1, "a1")]));
    env.logs
        .script()
        .stream_logs
        .push_err(OxiError::forbidden("pods/log is forbidden"));
    let request = ExcerptRequest::new(ExcerptSource::Workload(AggregateSpec::selector(
        "default", "app=web",
    )));
    let excerpt = read(&mut env, &request).unwrap();
    assert_eq!(excerpt.lines, 1);
    assert_eq!(excerpt.failures.len(), 1, "{:?}", excerpt.failures);
    assert!(
        excerpt.failures[0].starts_with("web-b/app: "),
        "{:?}",
        excerpt.failures
    );
    assert!(
        excerpt
            .notes()
            .iter()
            .any(|n| n.starts_with("Could not read web-b/app"))
    );
}

#[test]
fn a_workload_read_opens_at_most_max_streams_containers_in_all() {
    let mut env = Env::new();
    env.service.set_max_streams(2);
    for name in ["web-a", "web-b", "web-c", "web-d", "web-e"] {
        env.resources.insert(web_pod(name));
        env.script(Timeline::immediate([line(name, 1, "hello")]));
    }
    let request = ExcerptRequest::new(ExcerptSource::Workload(AggregateSpec::selector(
        "default", "app=web",
    )));
    let excerpt = read(&mut env, &request).unwrap();
    assert_eq!(
        env.logs.recorded_calls().len(),
        2,
        "a stream that ends must not free a slot for a pod the cap left out"
    );
    assert_eq!(excerpt.streams, 2);
    assert_eq!(excerpt.matched_pods, Some(5));
    assert_eq!(excerpt.skipped_pods, 3);
    assert!(
        excerpt.notes().iter().any(|n| n.contains('3')),
        "{:?}",
        excerpt.notes()
    );
}
