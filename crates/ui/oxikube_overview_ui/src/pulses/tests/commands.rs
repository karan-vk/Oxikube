//! `cluster::ShowPulses`: registered on the bus with a tool stub, queued for the window, opening
//! the page in the cluster's tab; and the sidebar entry that sends it.

use futures::StreamExt as _;
use gpui::TestAppContext;
use oxikube_app::ClusterSessionManager;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::MutationGuard;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_ui::IconName;
use oxikube_workspace::sidebar::{
    SectionBody, SidebarEntry, SidebarRegistry, SidebarSection, SidebarTarget,
};

use super::{Fixture, cluster, no_grace};
use crate::pulses::{PulsesRequestSink, register_commands, register_sidebar};

fn bus(sink: PulsesRequestSink) -> CommandBus {
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, sink).expect("registers once");
    let clock = std::sync::Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        std::sync::Arc::new(FakeClusterConnectorPort::new()),
        std::sync::Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    CommandBus::new(
        registry,
        MutationGuard::new(sessions, std::sync::Arc::new(FakeStatePort::new()), clock),
    )
}

#[test]
fn show_pulses_is_a_declared_read_command_with_a_palette_title() {
    let meta = command::lookup(CommandId::CLUSTER_SHOW_PULSES).expect("declared");
    assert_eq!(meta.title, "Show Pulses");
    assert!(!Command::ClusterShowPulses { cluster: cluster() }.is_mutating());
    assert_eq!(
        CommandId::CLUSTER_SHOW_PULSES.as_str(),
        "cluster::ShowPulses"
    );
}

#[test]
fn the_command_registers_a_tool_stub() {
    let (sink, _rx) = PulsesRequestSink::channel();
    let bus = bus(sink);
    let tool = bus
        .tool(CommandId::CLUSTER_SHOW_PULSES)
        .expect("a tool stub");
    assert_eq!(tool.name.as_str(), "app.cluster_show_pulses");
}

#[test]
fn the_handler_queues_the_cluster_for_the_window() {
    let (sink, mut rx) = PulsesRequestSink::channel();
    let bus = bus(sink);
    let outcome = futures::executor::block_on(bus.dispatch(
        Command::ClusterShowPulses { cluster: cluster() },
        DispatchContext::new(Initiator::Command, "palette"),
    ))
    .expect("dispatched");
    assert!(matches!(outcome, Outcome::Completed(_)));
    assert_eq!(futures::executor::block_on(rx.next()), Some(cluster()));
}

#[test]
fn a_second_registration_is_refused() {
    let (sink, _rx) = PulsesRequestSink::channel();
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, sink.clone()).expect("first");
    assert!(register_commands(&mut registry, sink).is_err());
}

#[test]
fn a_sink_whose_window_is_gone_fails_the_command_visibly() {
    let (sink, rx) = PulsesRequestSink::channel();
    drop(rx);
    assert!(sink.send(cluster()).is_err());
}

#[gpui::test]
fn the_sidebar_entry_joins_the_metrics_section_and_runs_the_command(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, no_grace());
    let entries = fx.vcx.update(|_, cx| {
        // No section yet: nothing is added, and it says so.
        assert!(!register_sidebar(cx));
        SidebarRegistry::register(
            cx,
            SidebarSection::new("metrics", "Metrics", IconName::ChartLine, 100).with_entries([
                SidebarEntry {
                    id: "provider".into(),
                    title: "Provider and settings".into(),
                    requires: Vec::new(),
                    target: None,
                    kind: None,
                },
            ]),
        );
        assert!(register_sidebar(cx));
        assert!(register_sidebar(cx), "idempotent");
        SidebarRegistry::sections(cx)
            .into_iter()
            .find(|s| s.id.as_ref() == "metrics")
            .map(|s| match s.body {
                SectionBody::Entries(entries) => entries,
                SectionBody::CustomResources => Vec::new(),
            })
            .unwrap_or_default()
    });
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_ref()).collect();
    assert_eq!(ids, ["provider", "pulses"], "one entry, added once");
    let pulses = entries
        .iter()
        .find(|e| e.id.as_ref() == "pulses")
        .expect("pulses");
    assert_eq!(pulses.title.as_ref(), "Pulses");
    assert_eq!(
        pulses.target,
        Some(SidebarTarget::Command(CommandId::CLUSTER_SHOW_PULSES))
    );
}

#[gpui::test]
fn the_window_opens_the_page_in_the_clusters_tab_once_and_brings_it_forward(
    cx: &mut TestAppContext,
) {
    use std::rc::Rc;

    use oxikube_testkit::FakeStatePort;
    use oxikube_workspace::{ClusterTabs, ClusterTabsDeps};

    use crate::pulses::{ITEM_KEY, PulsesViews, PulsesViewsDeps};

    let mut fx = Fixture::open(cx, no_grace());
    let ws = fx.ws.clone();
    let sessions = fx.sessions.clone();
    let tabs = fx.vcx.update(|window, cx| {
        oxikube_workspace::cluster_tab::init(cx);
        let deps = ClusterTabsDeps::new(
            sessions.clone(),
            std::sync::Arc::new(FakeStatePort::new()),
            Rc::new(fx.recorder.clone()),
        );
        ClusterTabs::start(&ws, deps, window, cx)
    });
    futures::executor::block_on(sessions.reconnect(&cluster())).expect("reconnect");
    fx.vcx.run_until_parked();
    let views = PulsesViewsDeps {
        page: fx.deps.clone(),
        tabs: tabs.downgrade(),
    };
    for _ in 0..2 {
        let opened = fx
            .vcx
            .update(|window, cx| PulsesViews::open(&views, &cluster(), window, cx));
        assert!(opened);
        fx.vcx.run_until_parked();
    }
    fx.vcx.update(|_, cx| {
        let tabs = tabs.read(cx);
        assert_eq!(
            tabs.active(),
            Some(&cluster()),
            "the cluster's tab is forward"
        );
        let workspace = tabs
            .tab(&cluster())
            .expect("a tab")
            .read(cx)
            .workspace()
            .clone();
        let pages = workspace
            .read(cx)
            .items()
            .filter(|item| item.item_key(cx).as_deref() == Some(ITEM_KEY))
            .count();
        assert_eq!(pages, 1, "opening it again shows the open one");
    });
    let other = oxikube_domain::ids::ClusterId::new(
        "/home/me/.kube/config",
        &oxikube_domain::ids::ContextName::new("zzz"),
    );
    let missing = fx
        .vcx
        .update(|window, cx| PulsesViews::open(&views, &other, window, cx));
    assert!(
        !missing,
        "a cluster with no tab here is reported, not invented"
    );
}
