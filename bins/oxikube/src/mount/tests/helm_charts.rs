//! The Helm charts catalog in the running app (E16-S08): the commands on the bus with their tool
//! stubs, the sidebar's Helm > Charts entry and `helm::OpenCharts` opening the page in the
//! connected cluster's tab, and the page's Install and Repositories buttons running on the bus.

use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::helm::{ChartHit, ChartRef};
use oxikube_helm_ui::charts::HelmChartsView;

use super::helm_repos::{fake_with, run, start};
use crate::app_state::AppState;
use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_testkit::HelmCall;

fn hits() -> Vec<ChartHit> {
    vec![
        ChartHit {
            chart: ChartRef::new("nginx", "15.0.0").in_repo("bitnami"),
            app_version: Some("1.25.0".into()),
            description: "A web server".into(),
        },
        ChartHit {
            chart: ChartRef::new("nginx", "14.0.0").in_repo("bitnami"),
            app_version: Some("1.24.0".into()),
            description: "A web server".into(),
        },
    ]
}

fn pages(app: &mut super::App) -> usize {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<HelmChartsView>()
            .len()
    })
}

#[gpui::test]
fn the_four_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = start(cx, &fake_with(&[]));
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, owner, tool) in [
        (
            CommandId::HELM_OPEN_CHARTS,
            "oxikube_helm_ui::charts",
            "helm.open_charts",
        ),
        (
            CommandId::HELM_SEARCH_CHARTS,
            "oxikube_app::helm_charts",
            "helm.search_charts",
        ),
        (
            CommandId::HELM_SHOW_CHART,
            "oxikube_app::helm_charts",
            "helm.show_chart",
        ),
        (
            CommandId::HELM_INSTALL_CHART,
            "oxikube_app::helm_charts",
            "helm.install_chart",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn the_sidebar_entry_opens_the_catalog_and_its_buttons_run_commands(cx: &mut TestAppContext) {
    let fake = fake_with(&["bitnami"]);
    fake.script().search.push_ok(hits());
    fake.script()
        .show
        .push_ok("# nginx\n\nA web server.".to_owned());
    let mut app = start(cx, &fake);
    app.press("enter");
    assert!(
        app.drawn("sidebar-entry-helm/charts"),
        "the Charts entry is in the Helm section"
    );
    assert_eq!(pages(&mut app), 0, "nothing opens by itself");

    app.click("sidebar-entry-helm/charts");
    assert_eq!(pages(&mut app), 1, "one page in the cluster's tab");
    assert!(app.drawn("helm-charts"));
    assert!(app.drawn("helm-chart-card:bitnami/nginx"));

    // Asking again (the palette's command) shows the open page.
    assert!(run(&mut app, Command::HelmOpenCharts));
    assert_eq!(pages(&mut app), 1);

    app.click("helm-chart-card:bitnami/nginx");
    assert!(app.drawn("helm-chart-readme"));
    assert!(
        fake.recorded_calls()
            .iter()
            .any(|c| matches!(c, HelmCall::Show(..)))
    );

    // Install runs `helm::InstallChart` on the real bus: it records the request for the install tab.
    app.click("helm-chart-install");
    app.vcx.run_until_parked();
    assert!(!app.drawn("helm-charts-problem"));
}

#[gpui::test]
fn an_agent_may_search_and_show_charts_through_the_bus(cx: &mut TestAppContext) {
    let fake = fake_with(&["bitnami"]);
    fake.script().search.push_ok(hits());
    let mut app = start(cx, &fake);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let out = block_on(bus.dispatch(
        Command::HelmSearchCharts {
            keyword: "nginx".into(),
        },
        DispatchContext::new(Initiator::Agent, "agent"),
    ))
    .expect("a read-only search is allowed")
    .completed()
    .expect("it ran");
    assert_eq!(out.data.expect("data")["charts"][0]["latest"], "15.0.0");
}
