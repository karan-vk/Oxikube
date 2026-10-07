//! `@logs` through the `ContextRegistry`, over fake ports.

use std::sync::Arc;

use oxikube_domain::agent::ContextBlock;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{ContextScope, LogSince};
use oxikube_testkit::{LogCall, Timeline, deployment};

use super::LogContextProvider;
use crate::context::ContextRegistry;
use crate::logs::excerpt::harness::{Env, FixedCluster, cluster_id, line, web_pod};

fn registry(env: &Env) -> ContextRegistry {
    let registry = ContextRegistry::new();
    registry
        .register(Arc::new(LogContextProvider::new(
            env.clusters(),
            env.service.clone(),
        )))
        .unwrap();
    registry
}

fn resolve(env: &mut Env, mention: &str, scope: &ContextScope) -> OxiResult<Vec<ContextBlock>> {
    let registry = registry(env);
    env.run(async { registry.resolve_text(mention, scope).await })
}

fn scope() -> ContextScope {
    ContextScope::new().with_cluster(cluster_id())
}

#[test]
fn a_mention_resolves_to_a_block_for_the_pod_with_its_source() {
    let mut env = Env::new();
    env.script(env.lines(3));
    let blocks = resolve(&mut env, "@logs/prod/web-0", &scope()).unwrap();
    let [block] = &blocks[..] else {
        panic!("one block: {blocks:?}")
    };
    assert_eq!(block.title, "Logs prod/web-0 (3 lines)");
    assert_eq!(block.mime, "text/plain");
    for needle in [
        "# cluster: kind (",
        "# namespace: prod",
        "# source: web-0",
        "# lines: 3",
        "# time: 2025-10-09T08:53:20.000Z to 2025-10-09T08:53:20.002Z",
        "2025-10-09T08:53:20.001Z web-0/app line 1\n",
    ] {
        assert!(block.body.contains(needle), "{needle}\n{}", block.body);
    }
    assert!(!block.truncated);
    let calls = env.logs.recorded_calls();
    let [
        LogCall::StreamLogs {
            namespace,
            pod,
            options,
        },
    ] = &calls[..]
    else {
        panic!()
    };
    assert_eq!((namespace.as_str(), pod.as_str()), ("prod", "web-0"));
    assert!(!options.follow);
}

#[test]
fn since_is_honoured_and_named_in_the_title() {
    let mut env = Env::new();
    env.script(env.lines(2));
    let blocks = resolve(&mut env, "@logs/prod/web-0/--since/10m", &scope()).unwrap();
    assert_eq!(blocks[0].title, "Logs prod/web-0 (2 lines, since 10m)");
    let calls = env.logs.recorded_calls();
    let [LogCall::StreamLogs { options, .. }] = &calls[..] else {
        panic!()
    };
    assert_eq!(options.since, Some(LogSince::Seconds(600)));
}

#[test]
fn tail_container_and_grep_options_apply() {
    let mut env = Env::new();
    env.script(Timeline::immediate((0..20).map(|i| {
        line(
            "web-0",
            i,
            if i % 2 == 0 { "error even" } else { "fine odd" },
        )
    })));
    let blocks = resolve(
        &mut env,
        "@logs/prod/web-0/sidecar/--tail=3/--grep=error",
        &scope(),
    )
    .unwrap();
    let body = &blocks[0].body;
    assert_eq!(body.matches("error even").count(), 3, "{body}");
    assert!(!body.contains("fine odd"));
    assert!(body.contains("# container: sidecar"));
    assert!(blocks[0].truncated, "seven older matches were left out");
    assert!(
        body.contains("# note: 7 older matching lines were omitted"),
        "{body}"
    );
    let calls = env.logs.recorded_calls();
    let [LogCall::StreamLogs { options, .. }] = &calls[..] else {
        panic!()
    };
    assert_eq!(options.container.as_deref(), Some("sidecar"));
}

#[test]
fn over_budget_output_carries_a_truncation_note_and_stays_in_budget() {
    let mut env = Env::new();
    env.script(Timeline::immediate(
        (0..400).map(|i| line("web-0", i, &format!("{i:0>120}"))),
    ));
    let scope = scope().with_max_total_bytes(6_000);
    let blocks = resolve(&mut env, "@logs/prod/web-0/--tail=400", &scope).unwrap();
    let block = &blocks[0];
    assert!(block.body.len() <= 6_000, "{}", block.body.len());
    assert!(block.truncated);
    assert!(block.body.contains("# note: "), "{}", &block.body[..600]);
    assert!(block.body.contains("size limit"), "{}", &block.body[..600]);
    assert!(
        block.body.contains(&format!("{:0>120}\n", 399)),
        "the newest line is kept"
    );
}

#[test]
fn a_workload_mention_merges_its_pods() {
    let mut env = Env::new();
    env.resources
        .insert(deployment().name("web").namespace("default").build());
    env.resources.insert(web_pod("web-a"));
    env.resources.insert(web_pod("web-b"));
    env.script(Timeline::immediate([line("web-a", 10, "a10")]));
    env.script(Timeline::immediate([line("web-b", 5, "b5")]));
    let blocks = resolve(&mut env, "@logs/deployment/default/web", &scope()).unwrap();
    assert_eq!(blocks[0].title, "Logs default/deployment/web (2 lines)");
    let body = &blocks[0].body;
    assert!(
        body.find("b5").unwrap() < body.find("a10").unwrap(),
        "merged by timestamp\n{body}"
    );
    assert!(body.contains("# source: deployment/web"));
}

#[test]
fn the_scopes_default_namespace_stands_for_a_missing_one() {
    let mut env = Env::new();
    env.script(env.lines(1));
    let scope = scope().with_default_namespace("staging");
    let blocks = resolve(&mut env, "@logs/web-0", &scope).unwrap();
    assert_eq!(blocks[0].title, "Logs staging/web-0 (1 lines)");
    let err = resolve(&mut env, "@logs/web-0", &ContextScope::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn secrets_never_reach_the_block() {
    let mut env = Env::new();
    env.script(Timeline::immediate([line(
        "web-0",
        0,
        "login ok password=correct-horse-battery token=ghp_abcdefghijklmnopqrstuvwxyz0123456789 Bearer abcdefghijklmnop.qrstuv",
    )]));
    let blocks = resolve(&mut env, "@logs/prod/web-0", &scope()).unwrap();
    for secret in [
        "correct-horse-battery",
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "abcdefghijklmnop.qrstuv",
    ] {
        assert!(!blocks[0].body.contains(secret), "{}", blocks[0].body);
    }
    assert!(
        blocks[0]
            .body
            .contains("# secrets are masked on a best-effort basis")
    );
}

#[test]
fn failures_surface_as_errors() {
    let mut env = Env::new();
    env.logs
        .script()
        .stream_logs
        .push_err(OxiError::not_found("pods \"ghost\" not found"));
    let err = resolve(&mut env, "@logs/prod/ghost", &scope()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);

    let err = resolve(&mut env, "@logs/prod/web-0/--tail=0", &scope()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);

    let registry = ContextRegistry::new();
    registry
        .register(Arc::new(LogContextProvider::new(
            Arc::new(FixedCluster(None)),
            env.service.clone(),
        )))
        .unwrap();
    let err = env
        .run(async {
            registry
                .resolve_text("@logs/prod/web-0", &ContextScope::new())
                .await
        })
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Network);
}

#[test]
fn the_provider_is_registered_under_logs() {
    let env = Env::new();
    assert_eq!(registry(&env).prefixes(), ["logs"]);
}
