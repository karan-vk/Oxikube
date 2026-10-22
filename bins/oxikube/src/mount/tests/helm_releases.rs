//! The Helm releases page in the running app (E16-S05): the commands on the bus with their tool
//! stubs, and the sidebar's Helm > Releases entry, `helm::OpenReleases` (the palette) and `:helm`
//! each opening the page, with the releases view under the banner, in the connected cluster's
//! tab. The release records are read natively through the session's resource port, with no
//! `helm` binary; decoding a record needs the Tokio runtime the test runtime lacks, so the data
//! itself is covered by `oxikube_app::helm_releases` (over `FakeHelmPort`), the page by
//! `oxikube_helm_ui::releases` (over a fake backend) and the reader by `oxikube_helm`'s kind tests.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_helm_ui::HelmPage;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn pages(app: &mut App) -> Vec<gpui::Entity<HelmPage>> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<HelmPage>())
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
    for (id, owner, tool) in [
        (
            CommandId::HELM_OPEN_RELEASES,
            "oxikube_helm_ui::releases",
            "helm.open_releases",
        ),
        (
            CommandId::HELM_SHOW_VALUES,
            "oxikube_app::helm_releases",
            "helm.show_values",
        ),
        (
            CommandId::HELM_SHOW_MANIFEST,
            "oxikube_app::helm_releases",
            "helm.show_manifest",
        ),
        (
            CommandId::HELM_REVEAL_VALUES,
            "oxikube_app::helm_releases",
            "helm.reveal_values",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn the_sidebar_entry_opens_the_releases_in_the_cluster_tab(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    assert!(app.drawn("sidebar-entry-helm/releases"));
    assert!(pages(&mut app).is_empty(), "nothing opens by itself");

    app.click("sidebar-entry-helm/releases");
    app.tick();
    app.tick();
    assert_eq!(pages(&mut app).len(), 1, "one page in the cluster's tab");
    assert!(app.drawn("helm-page"));
    assert!(
        app.drawn("helm-releases"),
        "the releases view under the banner"
    );
    assert!(app.drawn("helm-releases-table"));
    assert!(app.drawn("helm-releases-filter"));
    // No release records in the seeded cluster, and no helm binary: the view works and says so.
    assert!(app.drawn("helm-releases-empty"));
    assert!(app.drawn("helm-banner"));
}

#[gpui::test]
fn the_palettes_command_shows_the_open_page_instead_of_a_second(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    run(
        &mut app,
        Command::HelmOpenReleases {
            cluster: TestPorts::cluster_id(),
        },
    );
    app.tick();
    assert_eq!(pages(&mut app).len(), 1);
    run(
        &mut app,
        Command::HelmOpenReleases {
            cluster: TestPorts::cluster_id(),
        },
    );
    app.tick();
    assert_eq!(pages(&mut app).len(), 1, "asking again shows the open page");
}

#[gpui::test]
fn colon_helm_opens_the_page_from_the_jump_bar(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    app.press(":");
    app.tick();
    app.vcx.simulate_input("helm");
    app.vcx.run_until_parked();
    app.press("enter");
    app.tick();
    assert_eq!(pages(&mut app).len(), 1, "`:helm` opened the page");
    assert!(app.drawn("helm-releases"));
}

#[gpui::test]
fn show_values_of_a_release_that_does_not_exist_is_a_clean_error(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(
        Command::HelmShowValues {
            cluster: TestPorts::cluster_id(),
            namespace: "default".into(),
            name: "nope".into(),
            revision: None,
            all: false,
        },
        DispatchContext::new(Initiator::Agent, "agent"),
    ));
    app.vcx.run_until_parked();
    // A read-only command is allowed for an agent; the release is not there.
    let error = outcome.expect_err("not found");
    assert!(error.to_string().contains("nope"), "{error}");
}
