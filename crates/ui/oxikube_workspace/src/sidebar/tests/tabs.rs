//! The sidebar as the cluster tabs host it: one per cluster, in the cluster's own left dock.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::{ClusterSessionManager, IntegrationRegistry};
use oxikube_domain::access::AccessRules;
use oxikube_domain::command::Command;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{can_list, context, id};
use crate::sidebar::{SidebarDeps, SidebarPanel, tab_setup};
use crate::test_support::open_workspace;
use crate::{ClusterTabs, ClusterTabsDeps, CommandDispatcher};

#[derive(Default)]
struct Ignore(RefCell<Vec<Command>>);

impl CommandDispatcher for Ignore {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

#[gpui::test]
fn every_cluster_tab_gets_its_own_sidebar_with_its_own_permissions(cx: &mut TestAppContext) {
    let source =
        Arc::new(FakeClusterSourcePort::new().with_contexts([context("prod"), context("lab")]));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let sessions = ClusterSessionManager::new(
        connector.clone(),
        source,
        Arc::new(FakeClockPort::default()),
    );
    // prod: a restricted account; lab: cluster admin.
    connector
        .ports_for(&id("prod"))
        .access
        .set_rules(can_list(&[("", "pods")]));
    connector
        .ports_for(&id("lab"))
        .access
        .set_rules(AccessRules::all_access());

    let (ws, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_keymap::init_with_text("", oxikube_keymap::KeymapOptions::default(), cx);
        crate::cluster_tab::init(cx);
        crate::sidebar::init(cx);
    });
    let state = Arc::new(FakeStatePort::new());
    let deps = ClusterTabsDeps::new(sessions.clone(), state.clone(), Rc::new(Ignore::default()))
        .with_setup(tab_setup(SidebarDeps {
            sessions: sessions.clone(),
            integrations: IntegrationRegistry::new(),
            state,
        }));
    let tabs = vcx.update(|window, cx| ClusterTabs::start(&ws, deps, window, cx));
    vcx.run_until_parked();

    for name in ["prod", "lab"] {
        block_on(sessions.connect(&id(name))).expect("connect");
        vcx.run_until_parked();
    }

    let sections = |name: &str, vcx: &mut gpui::VisualTestContext| -> Vec<String> {
        let tab = vcx.update(|_, cx| tabs.read(cx).tab(&id(name)).cloned().expect("a tab"));
        vcx.update(|_, cx| {
            let workspace = tab.read(cx).workspace().clone();
            let panel = workspace
                .read(cx)
                .panel::<SidebarPanel>()
                .expect("the tab's sidebar");
            assert_eq!(panel.read(cx).cluster(), &id(name));
            // The sidebar is on screen as soon as the tab opens: its dock is open.
            let dock = workspace
                .read(cx)
                .dock(crate::DockPosition::Left, cx)
                .expect("a left dock");
            assert!(dock.is_open(), "{name}'s sidebar dock is open");
            panel.read(cx).visible_sections()
        })
    };
    assert_eq!(sections("prod", &mut vcx), ["cluster", "workloads"]);
    assert_eq!(sections("lab", &mut vcx).len(), 10);
}
