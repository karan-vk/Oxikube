//! The agent hooks of the main window (E08-S09): `@logs` in the context registry, `k8s.get_logs`
//! in the tool registry, both reading through the app's real sessions and log service, and the
//! log viewer's "Send to agent" reaching the queue through the real command bus.

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::context::CollectingConsumer;
use oxikube_domain::Capabilities;
use oxikube_domain::log::LogLine;
use oxikube_logs_ui::LogView;
use oxikube_ports::{ContextScope, ToolContext};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{TestPorts, Timeline};
use parking_lot::Mutex;
use serde_json::json;

use super::App;
use crate::app_state::{AgentHooks, AppState};

fn line(i: i64) -> LogLine {
    LogLine::new(
        jiff::Timestamp::from_second(1_791_115_200 + i).unwrap(),
        "web-running",
        "app",
        format!("hello {i}"),
    )
}

fn hooks(app: &mut App) -> AgentHooks {
    app.vcx
        .update(|_, cx| AppState::global(cx).agent_hooks().cloned())
        .expect("the mount built the agent hooks")
}

#[gpui::test]
fn the_mount_registers_the_log_provider_and_the_get_logs_tool(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let hooks = hooks(&mut app);
    assert_eq!(hooks.contexts.prefixes(), ["logs"], "@logs is registered");
    let defs = hooks.tools.defs();
    let def = defs
        .iter()
        .find(|def| def.name.as_str() == "k8s.get_logs")
        .expect("get_logs is registered");
    assert!(!def.is_mutating(), "read-only: no MutationGuard");
    assert!(
        hooks
            .tools
            .visible(Capabilities::LOGS)
            .iter()
            .any(|d| d.name == def.name),
        "offered to a session that can read logs"
    );
    assert_eq!(hooks.pending.pending(), 0);
}

#[gpui::test]
fn send_to_agent_in_the_log_view_reaches_the_queue_through_the_real_bus(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    ports
        .logs
        .script()
        .stream_logs
        .push_ok(Timeline::immediate((0..3).map(line)));
    app.open_pods_table();
    let ws = app.tab_workspace();
    let table = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    let targets = app.vcx.update(|_, cx| table.read(cx).action_targets(cx));
    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_action(
                oxikube_domain::command::CommandId::POD_VIEW_LOGS,
                targets,
                window,
                cx,
            );
        });
    });
    app.tick();
    app.tick();
    let view = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<LogView>().remove(0));

    let hooks = hooks(&mut app);
    app.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.request_send_to_agent(cx)));
    app.tick();
    assert_eq!(
        hooks.pending.pending(),
        1,
        "queued until the agent panel exists"
    );

    let consumer = CollectingConsumer::new();
    let _attached = hooks.pending.attach(consumer.clone());
    let [item] = &consumer.take()[..] else {
        panic!("one block")
    };
    assert_eq!(item.source.subject, "web-running");
    assert_eq!(item.source.lines, 3);
    assert!(item.block.body.contains("hello 2\n"), "{}", item.block.body);
    assert!(
        item.block.body.contains("# cluster: "),
        "{}",
        item.block.body
    );
}

#[gpui::test]
fn get_logs_and_at_logs_read_through_the_connected_cluster(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    for _ in 0..2 {
        ports
            .logs
            .script()
            .stream_logs
            .push_ok(Timeline::immediate((0..4).map(line)));
    }
    app.open_pods_table(); // connects the cluster
    let hooks = hooks(&mut app);

    let tool_result = Arc::new(Mutex::new(None));
    let mention_result = Arc::new(Mutex::new(None));
    app.vcx.update(|_, cx| {
        let (tools, out) = (hooks.tools.clone(), tool_result.clone());
        cx.spawn(async move |_| {
            let result = tools
                .invoke(
                    "k8s.get_logs",
                    json!({"pod": "web-running", "namespace": "default", "tail": 2}),
                    &ToolContext::agent(),
                )
                .await;
            *out.lock() = Some(result);
        })
        .detach();
        let (contexts, out) = (hooks.contexts.clone(), mention_result.clone());
        cx.spawn(async move |_| {
            let result = contexts
                .resolve_text("@logs/default/web-running/--since/5m", &ContextScope::new())
                .await;
            *out.lock() = Some(result);
        })
        .detach();
    });
    for _ in 0..4 {
        app.tick();
    }

    let output = tool_result
        .lock()
        .take()
        .expect("the tool call finished")
        .expect("a valid call");
    assert!(!output.is_error, "{output:?}");
    let text = output.content[0].as_text().unwrap();
    assert!(
        text.contains("hello 3") && text.contains("hello 2"),
        "{text}"
    );
    assert!(!text.contains("hello 0"), "tail 2: {text}");

    let blocks = mention_result
        .lock()
        .take()
        .expect("the mention resolved")
        .expect("a valid mention");
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].title.contains("since 5m"), "{}", blocks[0].title);
    assert!(blocks[0].body.contains("hello 0"));

    // Both reads were bounded reads, not follows.
    for call in ports.logs.recorded_calls() {
        let oxikube_testkit::LogCall::StreamLogs { options, .. } = call;
        assert!(!options.follow);
    }
}
