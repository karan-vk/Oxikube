//! The Helm upgrade tab in the running app (E16-S10): `helm::Upgrade` and `helm::UpgradeRelease` on
//! the bus with their tool stubs, the sidebar's Helm > Upgrade entry listing the releases, a picked
//! release opening its tab prefilled, and its preview running through the real bus and guard.

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::helm::{
    ChartHit, ChartRef, Release, ReleaseRef, ReleaseStatus, Revision, Values,
};
use oxikube_helm_ui::upgrade::{HelmUpgradeView, Stage};
use oxikube_testkit::HelmCall;

use super::helm_repos::{fake_with, run, start};
use crate::app_state::AppState;

fn tabs(app: &mut super::App) -> Vec<gpui::Entity<HelmUpgradeView>> {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<HelmUpgradeView>()
    })
}

fn script_release(fake: &oxikube_testkit::FakeHelmPort) {
    let script = fake.script();
    script.history.push_ok(vec![Revision {
        revision: 3,
        status: ReleaseStatus::Deployed,
        chart: ChartRef::new("nginx", "15.0.0"),
        app_version: Some("1.25.0".into()),
        updated: Timestamp::UNIX_EPOCH,
        description: String::new(),
    }]);
    script
        .values
        .push_ok(Values::from_yaml("replicaCount: 2\n").unwrap());
    script.search.push_ok(vec![ChartHit {
        chart: ChartRef::new("nginx", "15.4.0").in_repo("bitnami"),
        app_version: Some("1.25.3".into()),
        description: "A web server".into(),
    }]);
    script.show.push_ok("replicaCount: 1\n".to_owned());
    script.show.push_ok("replicaCount: 1\n".to_owned());
}

#[gpui::test]
fn upgrade_is_on_the_bus_with_its_tool_stubs(cx: &mut TestAppContext) {
    let mut app = start(cx, &fake_with(&[]));
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool) in [
        (CommandId::HELM_UPGRADE, "helm.upgrade"),
        (CommandId::HELM_UPGRADE_RELEASE, "helm.upgrade_release"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::helm_upgrade"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn the_sidebar_lists_the_releases_and_a_picked_one_opens_prefilled_and_previews_through_the_guard(
    cx: &mut TestAppContext,
) {
    let fake = fake_with(&["bitnami"]);
    let web = |cluster| ReleaseRef::new(cluster, "default", "web");
    let mut app = start(cx, &fake);
    app.press("enter");
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    let cluster = app.vcx.update(|_, cx| tab.read(cx).cluster().clone());
    fake.script().list_releases.push_ok(vec![Release::summary(
        web(cluster),
        ChartRef::new("nginx", "15.0.0"),
        3,
        ReleaseStatus::Deployed,
        Timestamp::UNIX_EPOCH,
    )]);
    script_release(&fake);

    assert!(tabs(&mut app).is_empty(), "nothing opens by itself");
    app.click("sidebar-entry-helm/upgrade");
    app.vcx.run_until_parked();
    assert_eq!(
        tabs(&mut app).len(),
        1,
        "the upgrade tab opens in the cluster's tab"
    );
    assert!(app.drawn("helm-upgrade-picker"));

    app.click("helm-upgrade-release:0");
    app.vcx.run_until_parked();
    let view = tabs(&mut app).pop().unwrap();
    assert_eq!(
        app.vcx.update(|_, cx| view.read(cx).stage().clone()),
        Stage::Editing
    );
    assert!(app.drawn("helm-upgrade-values"));
    let values = app.vcx.update(|_, cx| view.read(cx).values(cx));
    assert_eq!(values.expose(), "replicaCount: 2\n");

    let script = fake.script();
    script
        .manifest
        .push_ok("kind: Service\nmetadata:\n  name: web\n".to_owned());
    script.history.push_ok(vec![Revision {
        revision: 3,
        status: ReleaseStatus::Deployed,
        chart: ChartRef::new("nginx", "15.0.0"),
        app_version: None,
        updated: Timestamp::UNIX_EPOCH,
        description: String::new(),
    }]);
    script.template.push_ok(
        "kind: Service\nmetadata:\n  name: web\n---\nkind: Ingress\nmetadata:\n  name: web\n"
            .to_owned(),
    );
    script.upgrade.push_ok(Vec::new());
    app.click("helm-upgrade-preview");
    app.vcx.run_until_parked();
    assert!(app.drawn("helm-upgrade-manifest"), "the preview is shown");
    assert!(app.drawn("helm-upgrade-changes-added"));
    let calls = fake.recorded_calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, HelmCall::Template(r) if r.release == "web"))
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, HelmCall::Upgrade(r) if r.dry_run))
    );
    assert!(
        !calls
            .iter()
            .any(|c| matches!(c, HelmCall::Upgrade(r) if !r.dry_run)),
        "nothing is upgraded by a preview"
    );
}

#[gpui::test]
fn the_command_with_a_release_opens_its_tab(cx: &mut TestAppContext) {
    let fake = fake_with(&["bitnami"]);
    script_release(&fake);
    let mut app = start(cx, &fake);
    app.press("enter");
    assert!(run(
        &mut app,
        Command::HelmUpgradeRelease {
            namespace: Some("default".into()),
            release: Some("web".into()),
        }
    ));
    app.vcx.run_until_parked();
    let opened = tabs(&mut app);
    assert_eq!(opened.len(), 1);
    assert!(app.drawn("helm-upgrade"));
    // Asking again shows the open tab rather than a second one.
    assert!(run(
        &mut app,
        Command::HelmUpgradeRelease {
            namespace: Some("default".into()),
            release: Some("web".into()),
        }
    ));
    assert_eq!(tabs(&mut app).len(), 1);
}
