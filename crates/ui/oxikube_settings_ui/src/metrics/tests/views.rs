//! `cluster::ShowMetricsSettings`: the command queues the cluster, the window opens the page in
//! the cluster's own tab and brings it forward.

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::ClusterSessionManager;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::MutationGuard;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher};

use super::super::test_support::{RecordingDispatcher, ScriptedBackend, found};
use super::super::{
    ITEM_KEY, MetricsRequestSink, MetricsViews, MetricsViewsDeps, register_commands,
};

fn context(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext {
        server: Some(format!("https://{name}.example:6443")),
        ..ClusterContext::new(
            ClusterId::new("/home/me/.kube/config", &context),
            context,
            SourceId("kubeconfig".into()),
        )
    }
}

fn id(name: &str) -> ClusterId {
    context(name).cluster
}

#[test]
fn the_command_queues_the_cluster_and_has_a_tool_stub() {
    let (sink, mut requests) = MetricsRequestSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_settings_ui", |r| register_commands(r, sink))
        .unwrap();
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    let bus = CommandBus::new(registry, guard);

    let tool = bus
        .tool(oxikube_domain::command::CommandId::CLUSTER_SHOW_METRICS_SETTINGS)
        .expect("a tool stub");
    assert_eq!(tool.name.as_str(), "app.cluster_show_metrics_settings");

    let outcome = block_on(bus.dispatch(
        Command::ClusterShowMetricsSettings { cluster: id("a") },
        DispatchContext::new(Initiator::Command, "palette"),
    ))
    .unwrap();
    assert!(matches!(outcome, Outcome::Completed(_)));
    assert_eq!(block_on(requests.next()), Some(id("a")));
}

#[test]
fn a_sink_whose_window_is_gone_is_an_error() {
    let (sink, requests) = MetricsRequestSink::channel();
    drop(requests);
    assert!(sink.send(id("a")).is_err());
}

#[gpui::test]
fn the_page_opens_in_the_cluster_tab_once_and_brings_it_forward(cx: &mut TestAppContext) {
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context("a"), context("b")]));
    let clock = Arc::new(FakeClockPort::default());
    let sessions =
        ClusterSessionManager::new(Arc::new(FakeClusterConnectorPort::new()), source, clock);
    let (ws, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_keymap::init_with_text("", Default::default(), cx);
        oxikube_workspace::cluster_tab::init(cx);
    });
    let dispatcher = RecordingDispatcher::new();
    let inner: Rc<dyn CommandDispatcher> = Rc::new(dispatcher);
    let tabs = vcx.update(|window, cx| {
        let deps = ClusterTabsDeps::new(
            sessions.clone(),
            Arc::new(FakeStatePort::new()),
            inner.clone(),
        );
        ClusterTabs::start(&ws, deps, window, cx)
    });
    for name in ["a", "b"] {
        block_on(sessions.connect(&id(name))).expect("connect");
    }
    vcx.run_until_parked();
    // The last connected cluster's tab is the one shown; the page is asked for the other.
    let views = MetricsViewsDeps {
        backend: Rc::new(ScriptedBackend::new(found())),
        dispatcher: inner,
        tabs: tabs.downgrade(),
    };

    for _ in 0..2 {
        let opened = vcx.update(|window, cx| MetricsViews::open(&views, &id("a"), window, cx));
        assert!(opened);
        vcx.run_until_parked();
    }
    vcx.update(|_, cx| {
        let tabs = tabs.read(cx);
        assert_eq!(
            tabs.active(),
            Some(&id("a")),
            "the cluster's tab is forward"
        );
        let workspace = tabs.tab(&id("a")).unwrap().read(cx).workspace().clone();
        let pages = workspace
            .read(cx)
            .items()
            .filter(|item| item.item_key(cx).as_deref() == Some(ITEM_KEY))
            .count();
        assert_eq!(pages, 1, "opening it again shows the open one");
        let other = tabs.tab(&id("b")).unwrap().read(cx).workspace().clone();
        assert!(
            other
                .read(cx)
                .items()
                .all(|item| item.item_key(cx).as_deref() != Some(ITEM_KEY)),
            "the other cluster's tab is untouched"
        );
    });

    // A cluster with no tab in this window is reported, not invented.
    let missing = vcx.update(|window, cx| MetricsViews::open(&views, &id("zzz"), window, cx));
    assert!(!missing);
}
