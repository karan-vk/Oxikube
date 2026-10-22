//! The Helm install tab in the running app (E16-S09): `helm::Install` on the bus with its tool
//! stub, the catalog's Install opening the tab in the connected cluster's tab, and its preview
//! running through the real bus and guard.

use gpui::TestAppContext;
use oxikube_domain::command::CommandId;
use oxikube_domain::helm::{ChartHit, ChartRef};
use oxikube_helm_ui::install::HelmInstallView;
use oxikube_testkit::HelmCall;

use super::helm_repos::{fake_with, start};
use crate::app_state::AppState;

fn tabs(app: &mut super::App) -> usize {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<HelmInstallView>()
            .len()
    })
}

#[gpui::test]
fn install_is_on_the_bus_with_its_tool_stub(cx: &mut TestAppContext) {
    let mut app = start(cx, &fake_with(&[]));
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let id = CommandId::HELM_INSTALL;
    assert_eq!(bus.owner(id), Some("oxikube_app::helm_install"));
    assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some("helm.install"));
}

#[gpui::test]
fn the_catalogs_install_opens_the_tab_and_its_preview_runs_through_the_guard(
    cx: &mut TestAppContext,
) {
    let fake = fake_with(&["bitnami"]);
    fake.script().search.push_ok(vec![ChartHit {
        chart: ChartRef::new("nginx", "15.0.0").in_repo("bitnami"),
        app_version: Some("1.25.0".into()),
        description: "A web server".into(),
    }]);
    fake.script().show.push_ok("# nginx".to_owned());
    fake.script()
        .show
        .push_ok("# -- Number of replicas\nreplicaCount: 1\n".to_owned());
    let mut app = start(cx, &fake);
    app.press("enter");
    app.click("sidebar-entry-helm/charts");
    app.click("helm-chart-card:bitnami/nginx");
    assert_eq!(tabs(&mut app), 0, "nothing opens by itself");

    app.click("helm-chart-install");
    app.vcx.run_until_parked();
    assert_eq!(
        tabs(&mut app),
        1,
        "the install tab opens in the cluster's tab"
    );
    assert!(app.drawn("helm-install"));
    assert!(app.drawn("helm-install-values"));

    fake.script()
        .template
        .push_ok("apiVersion: v1\nkind: Service\nmetadata:\n  name: nginx\n".to_owned());
    fake.script().install.push_ok(Vec::new());
    // The fake cluster lists no namespace: `default` is created.
    app.click("helm-install-create-ns");
    app.click("helm-install-preview");
    app.vcx.run_until_parked();
    assert!(app.drawn("helm-install-manifest"), "the preview is shown");
    let calls = fake.recorded_calls();
    assert!(calls.iter().any(
        |c| matches!(c, HelmCall::Template(r) if r.chart.version == "15.0.0" && r.create_namespace)
    ));
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, HelmCall::Install(r) if r.dry_run))
    );
    assert!(
        !calls
            .iter()
            .any(|c| matches!(c, HelmCall::Install(r) if !r.dry_run)),
        "nothing is installed by a preview"
    );
}
