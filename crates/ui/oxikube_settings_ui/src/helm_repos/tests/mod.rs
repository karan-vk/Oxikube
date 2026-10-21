//! `helm::ShowRepos` and the sidebar entry.

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::MutationGuard;
use oxikube_app::{ClusterSessionManager, HelmReposService};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_helm_ui::repos::{ITEM_KEY, ServiceBackend};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeHelmPort, FakeStatePort,
};
use oxikube_workspace::sidebar::{
    SectionBody, SidebarRegistry, SidebarTarget, register_core_sections,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher};

use super::{
    ENTRY_ID, HelmReposRequestSink, HelmReposViews, HelmReposViewsDeps, SECTION_ID,
    register_commands, register_sidebar,
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

struct Nothing;

impl CommandDispatcher for Nothing {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn backend() -> Rc<ServiceBackend> {
    let fake = Arc::new(FakeHelmPort::new());
    let service = HelmReposService::new(fake.clone(), fake, Arc::new(FakeClockPort::default()));
    Rc::new(ServiceBackend::new(service))
}

#[test]
fn the_command_queues_a_request_and_has_a_tool_stub() {
    let (sink, mut requests) = HelmReposRequestSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_settings_ui::helm_repos", |r| {
            register_commands(r, sink)
        })
        .unwrap();
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    let bus = CommandBus::new(registry, guard);
    let tool = bus.tool(CommandId::HELM_SHOW_REPOS).expect("a tool stub");
    assert_eq!(tool.name.as_str(), "helm.show_repos");
    let outcome = block_on(bus.dispatch(
        Command::HelmShowRepos,
        DispatchContext::new(Initiator::Command, "palette"),
    ))
    .unwrap();
    assert!(matches!(outcome, Outcome::Completed(_)));
    assert_eq!(block_on(requests.next()), Some(()));
}

#[test]
fn a_sink_whose_window_is_gone_is_an_error() {
    let (sink, requests) = HelmReposRequestSink::channel();
    drop(requests);
    assert!(sink.send().is_err());
}

#[gpui::test]
fn the_sidebar_entry_joins_the_helm_section_and_runs_the_command(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(!register_sidebar(cx), "no Helm section yet");
        register_core_sections(cx);
        assert!(register_sidebar(cx));
        assert!(register_sidebar(cx), "idempotent");
        let section = SidebarRegistry::sections(cx)
            .into_iter()
            .find(|s| s.id.as_ref() == SECTION_ID)
            .expect("the Helm section");
        let SectionBody::Entries(all) = &section.body else {
            panic!("the Helm section holds entries");
        };
        let entries: Vec<_> = all.iter().filter(|e| e.id.as_ref() == ENTRY_ID).collect();
        assert_eq!(entries.len(), 1, "added once");
        assert_eq!(
            entries[0].target,
            Some(SidebarTarget::Command(CommandId::HELM_SHOW_REPOS))
        );
    });
}

#[gpui::test]
fn the_page_opens_in_the_active_cluster_tab_once(cx: &mut TestAppContext) {
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
    let inner: Rc<dyn CommandDispatcher> = Rc::new(Nothing);
    let tabs = vcx.update(|window, cx| {
        let deps = ClusterTabsDeps::new(
            sessions.clone(),
            Arc::new(FakeStatePort::new()),
            inner.clone(),
        );
        ClusterTabs::start(&ws, deps, window, cx)
    });
    let views = HelmReposViewsDeps {
        backend: backend(),
        dispatcher: inner,
        tabs: tabs.downgrade(),
    };
    // No cluster tab: nowhere to show the page, and nothing is invented.
    assert!(!vcx.update(|window, cx| HelmReposViews::open(&views, window, cx)));

    for name in ["a", "b"] {
        block_on(sessions.connect(&id(name))).expect("connect");
    }
    vcx.run_until_parked();
    let active = vcx.update(|_, cx| tabs.read(cx).active().cloned().expect("an active tab"));
    for _ in 0..2 {
        assert!(vcx.update(|window, cx| HelmReposViews::open(&views, window, cx)));
        vcx.run_until_parked();
    }
    vcx.update(|_, cx| {
        let tabs = tabs.read(cx);
        let workspace = tabs.tab(&active).unwrap().read(cx).workspace().clone();
        let pages = workspace
            .read(cx)
            .items()
            .filter(|item| item.item_key(cx).as_deref() == Some(ITEM_KEY))
            .count();
        assert_eq!(pages, 1, "opening it again shows the open one");
    });
}
