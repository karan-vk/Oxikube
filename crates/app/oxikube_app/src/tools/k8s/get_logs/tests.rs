//! `k8s.get_logs` through the `ToolRegistry`, over fake ports.

use std::sync::Arc;

use oxikube_domain::{Capabilities, ErrorKind, OxiError};
use oxikube_ports::{ContentPart, ToolContext, ToolOutput};
use oxikube_testkit::{LogCall, Timeline, deployment};
use serde_json::{Value, json};

use super::{GET_LOGS, GetLogsTool};
use crate::logs::LogCluster;
use crate::logs::excerpt::harness::{Env, FixedCluster, cluster_id, line, web_pod};
use crate::tools::ToolRegistry;

fn registry(env: &Env) -> ToolRegistry {
    let registry = ToolRegistry::new();
    GetLogsTool::register(&registry, env.clusters(), env.service.clone()).unwrap();
    registry
}

fn call(
    env: &mut Env,
    registry: &ToolRegistry,
    args: Value,
) -> oxikube_domain::OxiResult<ToolOutput> {
    let ctx = ToolContext::agent();
    env.run(async { registry.invoke(GET_LOGS, args, &ctx).await })
}

fn text(output: &ToolOutput) -> &str {
    output.content[0].as_text().expect("a text part")
}

#[test]
fn the_tool_is_registered_read_only_with_its_schema() {
    let env = Env::new();
    let registry = registry(&env);
    let def = registry
        .defs()
        .into_iter()
        .find(|d| d.name.as_str() == GET_LOGS)
        .expect("registered");
    assert!(!def.is_mutating(), "reading logs needs no MutationGuard");
    assert!(def.read_only_hint());
    assert_eq!(def.risk, None);
    assert_eq!(def.needs, Capabilities::LOGS);
    assert!(def.annotations.idempotent);
    def.validate().unwrap();
    let properties = def.input_schema["properties"].as_object().unwrap();
    for name in [
        "pod",
        "selector",
        "namespace",
        "container",
        "since",
        "tail",
        "grep",
    ] {
        assert!(properties.contains_key(name), "{name}");
    }
    assert_eq!(def.input_schema["additionalProperties"], json!(false));
    assert_eq!(registry.visible(Capabilities::LOGS).len(), 1);
    assert!(
        registry.visible(Capabilities::empty()).is_empty(),
        "hidden without the logs capability"
    );
    // A second registration of the same name is a wiring bug.
    assert!(GetLogsTool::register(&registry, env.clusters(), env.service).is_err());
}

#[test]
fn a_pod_call_returns_its_lines_with_server_time_and_pod() {
    let mut env = Env::new();
    env.script(env.lines(3));
    let registry = registry(&env);
    let out = call(
        &mut env,
        &registry,
        json!({"pod": "web-0", "namespace": "prod"}),
    )
    .unwrap();
    assert!(!out.is_error);
    assert_eq!(
        text(&out),
        "2025-10-09T08:53:20.000Z web-0/app line 0\n\
         2025-10-09T08:53:20.001Z web-0/app line 1\n\
         2025-10-09T08:53:20.002Z web-0/app line 2\n"
    );
    assert_eq!(out.structured.as_ref().unwrap()["lines"], 3);
    assert_eq!(out.structured.as_ref().unwrap()["truncated"], false);
    let calls = env.logs.recorded_calls();
    let [
        LogCall::StreamLogs {
            namespace,
            pod,
            options,
        },
    ] = &calls[..]
    else {
        panic!("{calls:?}")
    };
    assert_eq!((namespace.as_str(), pod.as_str()), ("prod", "web-0"));
    assert!(!options.follow, "the tool never follows");
}

#[test]
fn since_and_tail_reach_the_port_and_limit_the_answer() {
    let mut env = Env::new();
    env.script(env.lines(10));
    let registry = registry(&env);
    let out = call(
        &mut env,
        &registry,
        json!({"pod": "web-0", "since": "10m", "tail": 2}),
    )
    .unwrap();
    assert_eq!(out.structured.as_ref().unwrap()["lines"], 2);
    assert_eq!(out.structured.as_ref().unwrap()["omitted"], 8);
    assert_eq!(out.structured.as_ref().unwrap()["truncated"], true);
    assert!(
        text(&out)
            .starts_with("note: 8 older matching lines were omitted beyond the requested tail"),
        "{}",
        text(&out)
    );
    assert!(text(&out).contains("line 9"));
    assert!(!text(&out).contains("line 7"));
    let calls = env.logs.recorded_calls();
    let [LogCall::StreamLogs { options, .. }] = &calls[..] else {
        panic!()
    };
    assert_eq!(options.since, Some(oxikube_ports::LogSince::Seconds(600)));
}

#[test]
fn grep_filters_the_lines() {
    let mut env = Env::new();
    env.script(Timeline::immediate([
        line("web-0", 0, "GET /health 200"),
        line("web-0", 1, "POST /orders 500 Error: boom"),
        line("web-0", 2, "GET /orders 200"),
    ]));
    let registry = registry(&env);
    let out = call(
        &mut env,
        &registry,
        json!({"pod": "web-0", "grep": "error|5\\d\\d"}),
    )
    .unwrap();
    assert_eq!(out.structured.as_ref().unwrap()["lines"], 1);
    assert!(text(&out).contains("boom") && !text(&out).contains("/health"));
}

#[test]
fn a_selector_reads_every_pod_it_picks_merged_by_time() {
    let mut env = Env::new();
    env.resources.insert(web_pod("web-a"));
    env.resources.insert(web_pod("web-b"));
    env.script(Timeline::immediate([
        line("web-a", 10, "a10"),
        line("web-a", 30, "a30"),
    ]));
    env.script(Timeline::immediate([line("web-b", 20, "b20")]));
    let registry = registry(&env);
    let out = call(&mut env, &registry, json!({"selector": "app=web"})).unwrap();
    let order: Vec<_> = text(&out)
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(order, ["a10", "b20", "a30"]);
    assert_eq!(out.structured.as_ref().unwrap()["streams"], 2);
}

#[test]
fn a_workload_selector_resolves_through_the_object() {
    let mut env = Env::new();
    env.resources
        .insert(deployment().name("web").namespace("default").build());
    env.resources.insert(web_pod("web-a"));
    env.script(Timeline::immediate([line("web-a", 1, "hello from a")]));
    let registry = registry(&env);
    let out = call(&mut env, &registry, json!({"selector": "deployment/web"})).unwrap();
    assert!(text(&out).contains("hello from a"));
}

#[test]
fn pod_and_selector_together_or_neither_is_an_invalid_call() {
    let mut env = Env::new();
    let registry = registry(&env);
    for args in [json!({}), json!({"pod": "a", "selector": "app=b"})] {
        let err = call(&mut env, &registry, args.clone()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation, "{args}");
    }
    assert!(
        env.logs.recorded_calls().is_empty(),
        "nothing is read for an invalid call"
    );
}

#[test]
fn arguments_that_break_the_schema_are_refused_before_the_tool_runs() {
    let mut env = Env::new();
    let registry = registry(&env);
    for (args, needle) in [
        (json!({"pod": "a", "tail": 100_000}), "arguments.tail"),
        (json!({"pod": "a", "tail": 0}), "arguments.tail"),
        (json!({"pod": "a", "tail": "ten"}), "arguments.tail"),
        (json!({"pod": "a", "follow": true}), "unknown argument"),
        (json!({"pod": 5}), "arguments.pod"),
    ] {
        let err = call(&mut env, &registry, args.clone()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation, "{args}");
        assert!(err.message().contains(needle), "{args}: {}", err.message());
    }
    // `since` and `grep` pass the schema and fail in the tool.
    for args in [
        json!({"pod": "a", "since": "soon"}),
        json!({"pod": "a", "grep": "(open"}),
    ] {
        let err = call(&mut env, &registry, args.clone()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation, "{args}");
    }
}

#[test]
fn a_bearer_token_in_a_line_is_masked_in_the_tool_output() {
    let mut env = Env::new();
    env.script(Timeline::immediate([
        line(
            "web-0",
            0,
            "upstream call Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.e30.abcdefghijklmnop",
        ),
        line("web-0", 1, "api_key=AKIAABCDEFGHIJKLMNOP done"),
        line("web-0", 2, "ordinary"),
    ]));
    let registry = registry(&env);
    let out = call(&mut env, &registry, json!({"pod": "web-0"})).unwrap();
    let all = format!("{} {}", text(&out), out.structured.as_ref().unwrap());
    for secret in [
        "eyJhbGciOiJIUzI1NiJ9",
        "abcdefghijklmnop",
        "AKIAABCDEFGHIJKLMNOP",
    ] {
        assert!(!all.contains(secret), "{secret} leaked: {all}");
    }
    assert!(text(&out).contains("[redacted]") && text(&out).contains("ordinary"));
}

#[test]
fn a_missing_pod_is_an_error_the_model_sees_not_a_failed_call() {
    let mut env = Env::new();
    env.logs
        .script()
        .stream_logs
        .push_err(OxiError::not_found("pods \"ghost\" not found"));
    let registry = registry(&env);
    let out = call(&mut env, &registry, json!({"pod": "ghost"})).unwrap();
    assert!(out.is_error);
    assert!(text(&out).contains("ghost"), "{}", text(&out));
    let ContentPart::Text { .. } = &out.content[0] else {
        panic!()
    };
}

#[test]
fn no_connected_cluster_is_a_tool_error_and_an_unknown_one_too() {
    let mut env = Env::new();
    let registry = ToolRegistry::new();
    GetLogsTool::register(&registry, Arc::new(FixedCluster(None)), env.service.clone()).unwrap();
    let out = call(&mut env, &registry, json!({"pod": "web-0"})).unwrap();
    assert!(out.is_error);
    assert!(text(&out).contains("no cluster is connected"));

    let registry = ToolRegistry::new();
    let known = LogCluster {
        id: cluster_id(),
        title: "kind".into(),
        ports: env.ports(),
    };
    GetLogsTool::register(
        &registry,
        Arc::new(FixedCluster(Some(known))),
        env.service.clone(),
    )
    .unwrap();
    let other = oxikube_domain::ids::ClusterId::new(
        "~/.kube/config",
        &oxikube_domain::ids::ContextName::new("prod"),
    );
    let ctx = ToolContext::agent().with_cluster(other);
    let out = env
        .run(async {
            registry
                .invoke(GET_LOGS, json!({"pod": "web-0"}), &ctx)
                .await
        })
        .unwrap();
    assert!(out.is_error);
    assert!(text(&out).contains("no session"), "{}", text(&out));
}

#[test]
fn output_stays_within_the_size_cap_and_says_so() {
    let mut env = Env::new();
    env.script(Timeline::immediate(
        (0..2_000).map(|i| line("web-0", i, &format!("{i:0>300}"))),
    ));
    let registry = registry(&env);
    let out = call(&mut env, &registry, json!({"pod": "web-0", "tail": 2000})).unwrap();
    assert!(
        text(&out).len() <= crate::logs::MAX_EXCERPT_BYTES + 1_024,
        "{}",
        text(&out).len()
    );
    assert_eq!(out.structured.as_ref().unwrap()["truncated"], true);
    assert!(text(&out).contains("size limit"));
}
