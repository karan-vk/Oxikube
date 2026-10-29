//! The apply panel in the running app (E20-S05): `apply::Directory` and `apply::Kustomize` are on
//! the bus with tool stubs, and the command opens the panel in the connected cluster's tab, which
//! loads a directory of the (fake) file system, rehearses it against the cluster and applies it
//! through the guard once the user confirms.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_resources_ui::apply::{ApplyView, Stage};
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

const CONFIGMAP: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: web\ndata:\n  a: b\n";

fn panels(app: &mut App) -> Vec<gpui::Entity<ApplyView>> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ApplyView>())
}

fn run(app: &mut App, command: Command) {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me"))).expect("dispatched");
    app.vcx.run_until_parked();
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool) in [
        (CommandId::APPLY_DIRECTORY, "k8s.apply_directory"),
        (CommandId::APPLY_KUSTOMIZE, "k8s.apply_kustomize"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_resources_ui::apply"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn apply_directory_opens_the_panel_in_the_cluster_tab_and_applies_through_the_guard(
    cx: &mut TestAppContext,
) {
    let ports = TestPorts::seeded();
    ports.fs.insert("/work/manifests/web.yaml", CONFIGMAP);
    let resources = ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    assert!(panels(&mut app).is_empty(), "nothing opens by itself");

    run(
        &mut app,
        Command::ApplyDirectory {
            cluster: TestPorts::cluster_id(),
            path: Some("/work/manifests".into()),
            namespace: None,
        },
    );
    app.tick();
    app.tick();
    let panel = panels(&mut app).pop().expect("the panel opened in the tab");
    assert!(app.drawn("apply-panel"));
    app.vcx.update(|_, cx| {
        let model = panel.read(cx).model();
        assert_eq!(model.stage(), Stage::Review);
        assert_eq!(model.summary_line(), "1 created", "the server's dry run");
    });
    assert!(
        resources
            .mutating_calls()
            .iter()
            .all(|call| call.is_dry_run()),
        "reviewing wrote nothing"
    );

    app.click("apply-apply");
    app.click("apply-confirm");
    app.tick();
    app.tick();
    app.vcx.update(|_, cx| {
        let model = panel.read(cx).model();
        assert_eq!(model.stage(), Stage::Result);
        assert_eq!(model.summary_line(), "1 created");
    });
    assert!(
        resources
            .mutating_calls()
            .iter()
            .any(|call| !call.is_dry_run()),
        "the apply reached the cluster"
    );
    // The guard audited the plan and its object.
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let audited = block_on(state.ports().state.query_audit(&Default::default())).expect("audit");
    assert!(
        audited
            .records
            .iter()
            .any(|record| &*record.cmd == "resource::ApplyPlan"),
        "{:?}",
        audited.records
    );
}
