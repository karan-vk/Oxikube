//! Immediate commands on the real init path (E05-P600): the app's bus registers the cluster tab
//! commands and `namespace::Select` as immediate, and the runner the views use applies them in the
//! update that dispatched them: the session, the namespace selector and the tabs have moved before
//! any executor turn, so the frame drawn right after the input shows it.

use gpui::TestAppContext;
use oxikube_catalog_ui::namespaces::NamespaceSelector;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::TestPorts;
use oxikube_workspace::ClusterCommandRunner;

use super::App;
use crate::app_state::AppState;

#[gpui::test]
fn the_ui_only_commands_are_immediate_and_land_in_the_dispatching_update(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the mount set the bus").clone();
    for id in [
        CommandId::NAMESPACE_SELECT,
        CommandId::CLUSTER_SELECT,
        CommandId::CLUSTER_SWITCH_TAB,
        CommandId::CLUSTER_NEXT_TAB,
        CommandId::CLUSTER_PREVIOUS_TAB,
        CommandId::CLUSTER_CLOSE_TAB,
    ] {
        assert!(bus.runs_now(id), "{id} is immediate");
        assert!(bus.tool(id).is_some(), "{id} keeps its MCP tool stub");
    }
    for id in [
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
        CommandId::CLUSTER_CONNECT,
        CommandId::RESOURCE_DELETE,
        CommandId::CLUSTER_TOGGLE_READ_ONLY,
    ] {
        assert!(!bus.runs_now(id), "{id} stays async");
    }

    let cluster = TestPorts::cluster_id();
    let sessions = state.services().sessions.clone();
    let workspace = app.workspace();
    let runner = ClusterCommandRunner::new(bus, sessions.clone(), "me", &workspace);
    let tab = app.cluster_tabs()[0].clone();
    let label = app.vcx.update(|window, cx| {
        runner.run(
            Command::NamespaceSelect {
                cluster: cluster.clone(),
                namespaces: vec!["kube-system".into()],
            },
            window,
            cx,
        );
        // Still the dispatching update: the session has it.
        assert_eq!(
            sessions.get(&cluster).unwrap().namespace_selection(),
            &NamespaceSelection::single("kube-system")
        );
        tab.read(cx)
            .toolbar()
            .and_then(|view| view.clone().downcast::<NamespaceSelector>().ok())
    });
    // The update has ended (its effects ran), no executor turn yet: the selector shows it.
    let selector = label.expect("the tab's namespace selector");
    let shown = app.vcx.update(|_, cx| selector.read(cx).label());
    assert_eq!(shown, "kube-system");
}
