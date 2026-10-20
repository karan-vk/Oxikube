//! The agent's port-forward tools (E15-S08) and the `portforward.*` configuration, over the
//! manager rig: policy, permission, port range, per-cluster overrides, hot reload.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use oxikube_domain::Capabilities;
use oxikube_domain::command::Command;
use oxikube_domain::portforward::{ForwardPort, PortRange};
use oxikube_ports::{PortForwardError, ToolContext, ToolOutput};
use oxikube_testkit::PortForwardCall;
use serde_json::{Value, json};

use super::{Rig, cluster, pod, target};
use crate::portforward::commands::run;
use crate::portforward::{ForwardConfig, ForwardState};
use crate::tools::ToolRegistry;
use crate::tools::portforward::{
    DenyForwards, ForwardPermission, LIST_PORT_FORWARDS, START_PORT_FORWARD, StartRequest, install,
};

const ANY: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

/// Allows every start and counts the asks.
#[derive(Default)]
struct AllowAll(AtomicUsize);

#[async_trait]
impl ForwardPermission for AllowAll {
    async fn allows(&self, _: &ToolContext, _: &StartRequest) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed);
        true
    }
}

fn ctx() -> ToolContext {
    let mut ctx = ToolContext::agent();
    ctx.cluster = Some(cluster());
    ctx
}

fn args(extra: Value) -> Value {
    let mut base =
        json!({"kind": "pod", "name": "api-0", "namespace": "shop", "remote_port": 8080});
    base.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    base
}

fn rig_with(permission: Arc<dyn ForwardPermission>) -> (Rig, ToolRegistry) {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let tools = ToolRegistry::new();
    install(&tools, &rig.manager, permission).unwrap();
    (rig, tools)
}

async fn call(tools: &ToolRegistry, name: &str, args: Value) -> ToolOutput {
    tools.invoke(name, args, &ctx()).await.expect("a result")
}

fn text(out: &ToolOutput) -> String {
    format!("{:?}", out.content)
}

#[tokio::test]
async fn both_tools_are_registered_read_only_and_the_start_is_gated_by_capability() {
    let (_rig, tools) = rig_with(Arc::new(DenyForwards));
    let defs = tools.defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, [LIST_PORT_FORWARDS, START_PORT_FORWARD]);
    for def in &defs {
        def.validate().unwrap();
        assert!(!def.is_mutating(), "a forward changes no cluster object");
        assert_eq!(def.input_schema["additionalProperties"], false);
    }
    let list = &defs[0];
    let start = &defs[1];
    assert_eq!(list.needs, Capabilities::empty(), "no gate on a read");
    assert_eq!(start.needs, Capabilities::PORTFORWARD, "the start is gated");
    assert!(start.agent_exposed_by_default());
    assert_eq!(tools.visible(Capabilities::empty()).len(), 1);
    assert_eq!(tools.visible(Capabilities::PORTFORWARD).len(), 2);
}

#[tokio::test]
async fn installing_twice_names_the_taken_tool() {
    let (rig, tools) = rig_with(Arc::new(DenyForwards));
    assert!(install(&tools, &rig.manager, Arc::new(DenyForwards)).is_err());
}

#[tokio::test]
async fn list_reports_each_forward_with_its_url_and_status() {
    let (rig, tools) = rig_with(Arc::new(DenyForwards));
    let empty = call(&tools, LIST_PORT_FORWARDS, json!({})).await;
    assert_eq!(empty.structured.unwrap()["forwards"], json!([]));

    let info = rig
        .manager
        .start_and_wait(super::pod_spec("api-0", 8080))
        .await
        .unwrap();
    let out = call(&tools, LIST_PORT_FORWARDS, json!({})).await;
    let rows = out.structured.unwrap()["forwards"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["id"], info.id.0);
    assert_eq!(row["status"], "Active");
    assert_eq!(row["target"]["name"], "api-0");
    assert_eq!(row["target"]["kind"], "Pod");
    assert_eq!(row["remote_port"], "8080");
    assert_eq!(row["pod"], "api-0");
    assert_eq!(row["bind"], "127.0.0.1");
    let port = row["local_port"].as_u64().unwrap();
    assert_eq!(row["url"], format!("http://127.0.0.1:{port}"));
    assert_eq!(row["traffic"]["total_connections"], 0);

    let other = call(
        &tools,
        LIST_PORT_FORWARDS,
        json!({"cluster": "0000000000000000"}),
    )
    .await;
    assert_eq!(other.structured.unwrap()["forwards"], json!([]));
}

#[tokio::test]
async fn a_start_without_permission_is_refused_and_opens_nothing() {
    let (rig, tools) = rig_with(Arc::new(DenyForwards));
    let out = call(&tools, START_PORT_FORWARD, args(json!({}))).await;
    assert!(out.is_error, "{}", text(&out));
    assert!(text(&out).contains("permission"));
    assert!(rig.manager.list().is_empty());
    assert!(rig.port.recorded_calls().is_empty());
}

#[tokio::test]
async fn a_permitted_start_forwards_on_loopback_and_asks_once() {
    let allow = Arc::new(AllowAll::default());
    let (rig, tools) = rig_with(allow.clone());
    let out = call(&tools, START_PORT_FORWARD, args(json!({}))).await;
    assert!(!out.is_error, "{}", text(&out));
    let result = out.structured.unwrap();
    assert_eq!(result["status"], "Active");
    assert!(
        result["url"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:")
    );
    assert_eq!(allow.0.load(Ordering::Relaxed), 1);
    assert_eq!(rig.manager.list().len(), 1);
}

#[tokio::test]
async fn a_read_only_cluster_refuses_unless_the_setting_allows_it_and_never_asks_first() {
    let allow = Arc::new(AllowAll::default());
    let (rig, tools) = rig_with(allow.clone());
    *rig.backend.read_only.lock() = true;

    let out = call(&tools, START_PORT_FORWARD, args(json!({}))).await;
    assert!(out.is_error);
    assert!(text(&out).contains("allowed_in_readonly"), "{}", text(&out));
    assert_eq!(
        allow.0.load(Ordering::Relaxed),
        0,
        "no prompt for a refused start"
    );

    rig.manager.set_config(ForwardConfig {
        allowed_in_readonly: true,
        ..ForwardConfig::default()
    });
    let out = call(&tools, START_PORT_FORWARD, args(json!({}))).await;
    assert!(!out.is_error, "{}", text(&out));
}

#[tokio::test]
async fn a_cluster_override_beats_the_global_read_only_setting() {
    let (rig, tools) = rig_with(Arc::new(AllowAll::default()));
    *rig.backend.read_only.lock() = true;
    rig.manager.set_cluster_configs(HashMap::from([(
        cluster(),
        ForwardConfig {
            allowed_in_readonly: true,
            ..ForwardConfig::default()
        },
    )]));
    assert!(!rig.manager.config().allowed_in_readonly);
    assert!(rig.manager.config_for(&cluster()).allowed_in_readonly);
    let out = call(&tools, START_PORT_FORWARD, args(json!({}))).await;
    assert!(!out.is_error, "{}", text(&out));
}

#[tokio::test]
async fn a_non_loopback_bind_is_refused_unless_the_user_made_it_the_default() {
    let allow = Arc::new(AllowAll::default());
    let (rig, tools) = rig_with(allow.clone());

    let out = call(&tools, START_PORT_FORWARD, args(json!({"bind": "0.0.0.0"}))).await;
    assert!(out.is_error);
    assert!(text(&out).contains("default_bind"), "{}", text(&out));
    assert_eq!(allow.0.load(Ordering::Relaxed), 0);
    assert!(rig.manager.list().is_empty());

    rig.manager.set_config(ForwardConfig {
        default_bind: ANY,
        ..ForwardConfig::default()
    });
    let out = call(&tools, START_PORT_FORWARD, args(json!({"bind": "0.0.0.0"}))).await;
    assert!(!out.is_error, "{}", text(&out));
    assert_eq!(rig.manager.list()[0].spec.bind, ANY);
}

#[tokio::test]
async fn a_bad_bind_text_is_a_validation_error() {
    let (_rig, tools) = rig_with(Arc::new(AllowAll::default()));
    let err = tools
        .invoke(
            START_PORT_FORWARD,
            args(json!({"bind": "not-an-ip"})),
            &ctx(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::Validation);
}

#[tokio::test]
async fn local_ports_must_be_unprivileged_and_inside_the_range() {
    let (rig, tools) = rig_with(Arc::new(AllowAll::default()));
    let range = PortRange::new(30000, 30009);
    rig.manager.set_config(ForwardConfig {
        port_range: range,
        ..ForwardConfig::default()
    });

    let low = call(&tools, START_PORT_FORWARD, args(json!({"local_port": 80}))).await;
    assert!(
        low.is_error && text(&low).contains("privileged"),
        "{}",
        text(&low)
    );
    let outside = call(
        &tools,
        START_PORT_FORWARD,
        args(json!({"local_port": 40000})),
    )
    .await;
    assert!(
        outside.is_error && text(&outside).contains("port_range"),
        "{}",
        text(&outside)
    );
    assert!(rig.manager.list().is_empty());

    let inside = call(
        &tools,
        START_PORT_FORWARD,
        args(json!({"local_port": 30003})),
    )
    .await;
    assert!(!inside.is_error, "{}", text(&inside));
    assert_eq!(inside.structured.unwrap()["local_port"], 30003);
}

#[tokio::test]
async fn a_failed_start_carries_the_inline_suggestion() {
    let (rig, tools) = rig_with(Arc::new(AllowAll::default()));
    rig.port.fail_next_start(PortForwardError::BindFailed {
        addr: "127.0.0.1:9000".parse().unwrap(),
        kind: std::io::ErrorKind::AddrInUse,
    });
    let out = call(
        &tools,
        START_PORT_FORWARD,
        args(json!({"local_port": 9000})),
    )
    .await;
    assert!(out.is_error);
    assert!(text(&out).contains("already in use"), "{}", text(&out));
}

#[tokio::test]
async fn automatic_allocation_walks_the_range_past_busy_ports() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let range = PortRange::new(31000, 31019).unwrap();
    rig.manager.set_config(ForwardConfig {
        port_range: Some(range),
        ..ForwardConfig::default()
    });
    for _ in 0..2 {
        rig.port.fail_next_start(PortForwardError::BindFailed {
            addr: "127.0.0.1:31000".parse().unwrap(),
            kind: std::io::ErrorKind::AddrInUse,
        });
    }
    let info = rig
        .manager
        .start_and_wait(super::pod_spec("api-0", 8080))
        .await
        .unwrap();
    let ForwardState::Active { local_addr, .. } = info.state else {
        panic!("{:?}", info.state)
    };
    assert!(range.contains(local_addr.port()), "{local_addr}");
    let asked: Vec<u16> = rig
        .port
        .recorded_calls()
        .iter()
        .filter_map(|c| match c {
            PortForwardCall::Start { local_port, .. } => *local_port,
            PortForwardCall::Forward { .. } => None,
        })
        .collect();
    assert_eq!(asked.len(), 3, "two busy ports, then a free one: {asked:?}");
    assert!(asked.iter().all(|p| range.contains(*p)));
    assert_eq!(
        asked[1],
        range.nth_wrapping(u32::from(asked[0] - range.start()) + 1)
    );
}

#[tokio::test]
async fn without_a_range_the_os_picks() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    rig.manager
        .start_and_wait(super::pod_spec("api-0", 8080))
        .await
        .unwrap();
    assert!(matches!(
        rig.port.recorded_calls().as_slice(),
        [PortForwardCall::Start {
            local_port: None,
            ..
        }]
    ));
}

#[tokio::test]
async fn a_config_change_reaches_new_forwards_and_leaves_running_ones_alone() {
    let rig = Rig::new();
    rig.backend.push(
        "Pod",
        vec![pod("api-0", "Running"), pod("api-1", "Running")],
    );
    let first = rig
        .manager
        .start_and_wait(super::pod_spec("api-0", 8080))
        .await
        .unwrap();
    assert_eq!(first.spec.bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
    let starts_before = rig.port.recorded_calls().len();

    rig.manager.set_config(ForwardConfig {
        default_bind: ANY,
        ..ForwardConfig::default()
    });
    assert_eq!(
        rig.port.recorded_calls().len(),
        starts_before,
        "nothing restarted"
    );
    assert_eq!(
        rig.manager.get(first.id).unwrap(),
        first,
        "the running forward is untouched"
    );

    let out = run(
        &rig.manager,
        Command::PortForwardStart {
            target: target("Pod", "api-1"),
            remote_port: ForwardPort::Number(8080),
            local_port: None,
            bind: None,
        },
    )
    .await
    .unwrap();
    let id = out.data.unwrap()["id"].as_u64().unwrap();
    let second = rig.manager.get(crate::portforward::ForwardId(id)).unwrap();
    assert_eq!(
        second.spec.bind, ANY,
        "a new forward follows the new default"
    );
}
