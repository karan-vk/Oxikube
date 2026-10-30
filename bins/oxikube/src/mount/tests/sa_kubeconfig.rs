//! "Generate kubeconfig" in the running app (E17-S09), on the real init path with fake ports: the
//! Access Control section reaches the ServiceAccounts table, a row opens its detail with the
//! Kubeconfig section, the button sends `serviceaccount::GenerateKubeconfig` through the real bus
//! and guard (one confirmation, an audit record without the token), and the finished kubeconfig
//! shows masked in the panel. The command is on the bus with its tool stub and the row menu offers
//! it.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_app::sa_kubeconfig::KubeconfigVault;
use oxikube_domain::Resource;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::{ResourceCall, TestPorts, discovery_sets};
use oxikube_workspace::modal::ConfirmDialog;
use oxikube_workspace::sidebar::SidebarPanel;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

const TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.mount-test-token-payload.signature";

fn account() -> ResourceRef {
    ResourceRef::namespaced(
        TestPorts::cluster_id(),
        Gvk::new("", "v1", "ServiceAccount"),
        "shop",
        "builder",
    )
}

/// A connected cluster with the ServiceAccount `shop/builder` and a scripted TokenRequest.
fn start(cx: &mut TestAppContext) -> App {
    let ports = TestPorts::seeded();
    let cluster = ports.connector.ports_for(&TestPorts::cluster_id());
    cluster.discovery.set_kinds(discovery_sets::core());
    cluster.resources.insert(
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ServiceAccount",
            "metadata": {"name": "builder", "namespace": "shop", "uid": "uid-builder",
                         "resourceVersion": "2"},
        }))
        .expect("a ServiceAccount"),
    );
    cluster
        .resources
        .script()
        .create_subresource
        .push_ok(json!({
            "apiVersion": "authentication.k8s.io/v1", "kind": "TokenRequest",
            "status": {"token": TOKEN, "expirationTimestamp": "2026-10-28T13:00:00Z"},
        }));
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    app
}

fn token_requests(app: &App) -> Vec<ResourceCall> {
    app.cluster_ports()
        .resources
        .mutating_calls()
        .into_iter()
        .filter(|call| matches!(call, ResourceCall::CreateSubresource { .. }))
        .collect()
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub_and_a_row_action(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let id = CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG;
    assert_eq!(bus.owner(id), Some("oxikube_app::sa_kubeconfig"));
    assert_eq!(id.tool_name(), "k8s.serviceaccount_generate_kubeconfig");
    assert!(bus.tool(id).is_some(), "it has an MCP tool stub");
    let offered: Vec<_> = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .map(|spec| spec.command)
        .collect();
    assert!(
        offered.contains(&id),
        "the ServiceAccount row menu offers it"
    );
}

#[gpui::test]
fn the_service_account_panel_generates_a_kubeconfig_through_the_guard(cx: &mut TestAppContext) {
    let mut app = start(cx);
    // Access Control / Service Accounts, the first row, opened in the detail drawer.
    let ws = app.tab_workspace();
    let panel = app
        .vcx
        .update(|_, cx| ws.read(cx).panel::<SidebarPanel>().expect("the sidebar"));
    app.vcx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.activate("access-control/serviceaccounts", cx)
        })
    });
    app.tick();
    app.click("td-0-0");
    app.press("enter");
    app.tick();
    assert!(
        app.drawn("service-account-panel"),
        "the panel is in the drawer"
    );
    assert!(app.drawn("sa-kubeconfig"), "with its Kubeconfig section");
    assert!(
        !app.drawn("sa-kubeconfig-preview"),
        "nothing is generated yet"
    );

    app.click("sa-kubeconfig-generate");
    app.tick();
    // The guard asks first, naming the account; nothing was sent.
    let message = app
        .vcx
        .update(|_, cx| {
            let dialog = app_dialog(&ws, cx)?;
            Some(dialog)
        })
        .expect("a confirmation dialog");
    assert!(
        message.contains("shop/builder") && message.contains("1 hour"),
        "{message}"
    );
    assert!(
        token_requests(&app).is_empty(),
        "nothing is sent before the click"
    );
    app.press("enter");
    for _ in 0..4 {
        app.vcx.executor().advance_clock(Duration::from_millis(300));
        app.vcx.run_until_parked();
    }

    let requests = token_requests(&app);
    assert_eq!(requests.len(), 1);
    let ResourceCall::CreateSubresource { body, name, .. } = &requests[0] else {
        unreachable!()
    };
    assert_eq!(name, "builder");
    assert_eq!(body["spec"]["expirationSeconds"], 3600);
    assert!(
        app.drawn("sa-kubeconfig-preview"),
        "the kubeconfig is in the panel"
    );
    assert!(app.drawn("sa-kubeconfig-copy"));
    assert!(app.drawn("sa-kubeconfig-save"));
    assert!(KubeconfigVault::shared().get(&account()).is_some());

    // Audited as the account and the lifetime; the token is nowhere in the log.
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "serviceaccount::GenerateKubeconfig");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert!(!format!("{audit:?}").contains("mount-test-token-payload"));

    // Discarding drops it from the panel and the vault.
    app.click("sa-kubeconfig-discard");
    assert!(!app.drawn("sa-kubeconfig-preview"));
    assert!(KubeconfigVault::shared().get(&account()).is_none());
}

#[gpui::test]
fn a_read_only_cluster_refuses_for_the_user_and_an_agent_before_any_request(
    cx: &mut TestAppContext,
) {
    let mut app = start(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let sessions = state.services().sessions.clone();
    sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("the session");
    let bus = state.command_bus().expect("the bus");
    for initiator in [Initiator::Ui, Initiator::Agent] {
        let command = Command::ServiceAccountGenerateKubeconfig {
            target: account(),
            expiration_seconds: None,
            audiences: Vec::new(),
            allow_legacy_token: false,
        };
        let result = futures::executor::block_on(
            bus.dispatch(command, DispatchContext::new(initiator, "tester")),
        );
        assert!(result.is_err(), "{initiator:?} is refused: {result:?}");
    }
    assert!(token_requests(&app).is_empty());
    assert!(KubeconfigVault::shared().get(&account()).is_none());
}

/// The message of the confirmation dialog the cluster tab shows, if one is up.
fn app_dialog(ws: &gpui::Entity<oxikube_workspace::Workspace>, cx: &gpui::App) -> Option<String> {
    let dialog = ws
        .read(cx)
        .modal_layer()
        .read(cx)
        .active_modal::<ConfirmDialog>()?;
    Some(dialog.read(cx).message_text().to_string())
}
