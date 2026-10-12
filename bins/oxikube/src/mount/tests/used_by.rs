//! "Used by" in the running app (E12-S18), on the real init path with fake ports: the Config,
//! Access Control and Storage sections reach the tables of the five referenced kinds, `u` on a
//! row opens its detail on the Used by tab through the real bus, the tab lists the workloads whose
//! Pods name the object (a Deployment, found through its ReplicaSet) and a click opens it. The
//! command is on the bus with its tool stub and the row action is offered.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::{UsedBy, WorkloadUse};
use oxikube_domain::Resource;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_resources_ui::detail::{DetailDrawer, DetailTab, DetailView};
use oxikube_testkit::{TestPorts, discovery_sets};
use oxikube_workspace::sidebar::SidebarPanel;
use serde_json::{Value, json};

use super::App;
use crate::app_state::AppState;

fn object(api_version: &str, kind: &str, name: &str, extra: Value) -> Resource {
    let mut json = json!({
        "apiVersion": api_version, "kind": kind,
        "metadata": {"name": name, "namespace": "shop", "uid": format!("uid-{name}"),
                     "resourceVersion": "2"},
    });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        if key == "ownerReferences" {
            json["metadata"][key] = value;
        } else {
            json[key] = value;
        }
    }
    Resource::from_json(json).expect("a resource")
}

fn owned(kind: &str, name: &str, api_version: &str) -> Value {
    json!({"ownerReferences": [{"apiVersion": api_version, "kind": kind, "name": name,
                                "uid": format!("uid-{name}"), "controller": true}]})
}

fn pod(name: &str, spec: Value) -> Resource {
    let mut extra = owned("ReplicaSet", "web-5d", "apps/v1");
    extra["spec"] = spec;
    extra["status"] = json!({"phase": "Running"});
    object("v1", "Pod", name, extra)
}

/// A connected cluster with `shop/app` (ConfigMap), a ServiceAccount `builder` and a claim `data`,
/// all used by the Pods of the Deployment `web` (through its ReplicaSet `web-5d`).
fn start(cx: &mut TestAppContext) -> App {
    let ports = TestPorts::seeded();
    let cluster = ports.connector.ports_for(&TestPorts::cluster_id());
    cluster.discovery.set_kinds(discovery_sets::core());
    let spec = json!({
        "serviceAccountName": "builder",
        "priorityClassName": "urgent",
        "volumes": [
            {"name": "cfg", "configMap": {"name": "app"}},
            {"name": "data", "persistentVolumeClaim": {"claimName": "data"}},
        ],
    });
    for resource in [
        object("v1", "ConfigMap", "app", json!({"data": {"k": "v"}})),
        object("v1", "ServiceAccount", "builder", json!({})),
        object("v1", "PersistentVolumeClaim", "data", json!({})),
        Resource::from_json(json!({
            "apiVersion": "scheduling.k8s.io/v1", "kind": "PriorityClass",
            "metadata": {"name": "urgent", "uid": "uid-urgent", "resourceVersion": "2"},
            "value": 1000,
        }))
        .expect("a priority class"),
        object("apps/v1", "Deployment", "web", json!({})),
        object(
            "apps/v1",
            "ReplicaSet",
            "web-5d",
            owned("Deployment", "web", "apps/v1"),
        ),
        pod("web-5d-a", spec.clone()),
        pod("web-5d-b", spec),
    ] {
        cluster.resources.insert(resource);
    }
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    app
}

impl App {
    /// Opens the table behind the sidebar entry `id`, as a click on it does (rows below the fold
    /// are not drawn in a test window).
    fn open_entry(&mut self, id: &str) {
        let ws = self.tab_workspace();
        let panel = self
            .vcx
            .update(|_, cx| ws.read(cx).panel::<SidebarPanel>().expect("the sidebar"));
        self.vcx
            .update(|_, cx| panel.update(cx, |panel, cx| panel.activate(id, cx)));
        self.tick();
    }

    /// Lets the Pods feed land and the tab's polls (every 200 ms while it reads) pick it up.
    fn settle_used_by(&mut self) {
        for _ in 0..4 {
            self.vcx
                .executor()
                .advance_clock(Duration::from_millis(300));
            self.vcx.run_until_parked();
        }
    }
}

fn drawer(app: &mut App) -> gpui::Entity<DetailView> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| {
            ws.read(cx)
                .panel::<DetailDrawer>()
                .and_then(|drawer| drawer.read(cx).view().cloned())
        })
        .expect("the detail drawer is open")
}

fn answer(app: &mut App, view: &gpui::Entity<DetailView>) -> Vec<WorkloadUse> {
    match app
        .vcx
        .update(|_, cx| view.read(cx).used_by_answer().cloned())
    {
        Some(UsedBy::Known { workloads, .. }) => workloads,
        other => panic!("expected an answer, got {other:?}"),
    }
}

fn deployment() -> ResourceRef {
    ResourceRef::namespaced(
        TestPorts::cluster_id(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    )
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub_and_a_row_action(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let id = CommandId::RESOURCE_SHOW_USED_BY;
    assert_eq!(bus.owner(id), Some("oxikube_resources_ui"));
    assert_eq!(id.tool_name(), "k8s.resource_show_used_by");
    assert!(bus.tool(id).is_some(), "it has an MCP tool stub");
    let offered: Vec<_> = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .map(|spec| spec.command)
        .collect();
    assert!(offered.contains(&id));
}

#[gpui::test]
fn u_on_a_config_map_opens_its_used_by_tab_with_the_deployment_and_a_click_opens_it(
    cx: &mut TestAppContext,
) {
    let mut app = start(cx);
    app.open_entry("config/configmaps");
    app.click("td-0-0");
    app.press("u");
    app.settle_used_by();

    let view = drawer(&mut app);
    assert_eq!(
        app.vcx.update(|_, cx| view.read(cx).tab()),
        DetailTab::UsedBy
    );
    assert_eq!(
        answer(&mut app, &view),
        [WorkloadUse {
            target: deployment(),
            pods: 2
        }],
        "two Pods of one ReplicaSet are one Deployment"
    );
    assert!(app.drawn("detail-tab-used-by"));
    assert!(app.drawn("detail-used-by-summary"));
    assert!(app.drawn("used-by-row-0"));

    // A click opens the workload in the drawer, through the real bus.
    app.click("used-by-row-0");
    app.tick();
    let shown = drawer(&mut app);
    assert_eq!(
        app.vcx.update(|_, cx| shown.read(cx).target().clone()),
        deployment()
    );
}

#[gpui::test]
fn u_works_on_service_accounts_claims_and_priority_classes_too(cx: &mut TestAppContext) {
    let mut app = start(cx);
    for entry in [
        "access-control/serviceaccounts",
        "storage/persistentvolumeclaims",
        "config/priorityclasses",
    ] {
        app.open_entry(entry);
        app.click("td-0-0");
        app.press("u");
        app.settle_used_by();
        let view = drawer(&mut app);
        assert_eq!(
            app.vcx.update(|_, cx| view.read(cx).tab()),
            DetailTab::UsedBy,
            "{entry}"
        );
        assert_eq!(answer(&mut app, &view).len(), 1, "{entry}");
        assert_eq!(answer(&mut app, &view)[0].target, deployment(), "{entry}");
    }
}

#[gpui::test]
fn u_is_free_on_the_pods_table(cx: &mut TestAppContext) {
    let mut app = start(cx);
    app.open_entry("workloads/pods");
    app.click("td-0-0");
    app.press("u");
    app.tick();
    let ws = app.tab_workspace();
    let opened = app.vcx.update(|_, cx| {
        ws.read(cx)
            .panel::<DetailDrawer>()
            .is_some_and(|drawer| drawer.read(cx).view().is_some())
    });
    assert!(
        !opened,
        "nothing opens: `u` is bound to the referenced kinds only"
    );
}
