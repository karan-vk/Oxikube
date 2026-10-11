//! The Network section on the real init path (E12-S14): `ingressclass::SetDefault` is on the
//! app's bus with a tool stub and is a row action of IngressClass rows only; running it from a
//! table asks for a (low-risk) confirmation, then reaches the cluster through the app's own
//! `CommandBus` and `MutationGuard`, audited; a read-only cluster refuses it for every initiator;
//! the sidebar lists the Gateway API only where the cluster serves it.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::Resource;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{ResourceCall, TestPorts};
use oxikube_workspace::modal::DialogModal;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

const DEFAULT: &str = "ingressclass.kubernetes.io/is-default-class";

fn kind(group: &str, name: &str, plural: &str, namespaced: bool) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "patch", "delete"]),
        namespaced,
    }
}

fn class(name: &str, default: bool) -> Resource {
    let annotations = if default {
        json!({ DEFAULT: "true" })
    } else {
        json!({})
    };
    Resource::from_json(json!({
        "apiVersion": "networking.k8s.io/v1", "kind": "IngressClass",
        "metadata": {"name": name, "annotations": annotations, "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {"controller": "k8s.io/ingress-nginx"}
    }))
    .unwrap()
}

fn class_ref(name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(
        TestPorts::cluster_id(),
        Gvk::new("networking.k8s.io", "v1", "IngressClass"),
        name,
    )
}

impl App {
    /// An IngressClass table in the connected cluster's tab (`nginx`, not the default; `traefik`,
    /// the default), the first row under the cursor.
    fn open_ingress_classes(&mut self) -> gpui::Entity<ResourceTable> {
        self.serve([kind(
            "networking.k8s.io",
            "IngressClass",
            "ingressclasses",
            false,
        )]);
        let resources = self
            .ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources;
        resources.insert(class("nginx", false));
        resources.insert(class("traefik", true));
        self.press("enter");
        self.tick();
        self.click("sidebar-entry-network/ingressclasses");
        self.tick();
        let ws = self.tab_workspace();
        let table = self
            .vcx
            .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
        self.press("down");
        table
    }

    /// The message of the confirmation dialog the guard asked with, if one is open.
    fn class_confirmation(&mut self) -> Option<String> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            let dialog = ws
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()?;
            Some(dialog.read(cx).message_text()?.to_string())
        })
    }
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub_and_is_a_row_action_of_ingress_classes(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let id = CommandId::INGRESS_CLASS_SET_DEFAULT;
    assert_eq!(bus.owner(id), Some("oxikube_app::ingress_class"));
    let tool = bus.tool(id).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.ingressclass_set_default");
    let spec = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .find(|spec| spec.command == id)
        .copied()
        .expect("a row action");
    assert_eq!(spec.label, Some("Set as default"));
    assert!(!spec.bulk, "one class at a time");
}

#[gpui::test]
fn set_as_default_confirms_then_moves_the_flag_through_the_guard_and_the_audit_log(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let table = app.open_ingress_classes();
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    resources.script().patch.push_ok(class("nginx", true));
    resources.script().patch.push_ok(class("traefik", false));

    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            let targets = table.action_targets(cx);
            assert_eq!(targets, [class_ref("nginx")], "the cursor row");
            table.run_action(CommandId::INGRESS_CLASS_SET_DEFAULT, targets, window, cx);
        })
    });
    app.tick();
    let message = app.class_confirmation().expect("a confirmation dialog");
    assert!(
        message.contains("Set Default Ingress Class: IngressClass nginx"),
        "{message}"
    );
    assert!(
        message.contains("Clears the default flag of the other Ingress classes"),
        "{message}"
    );
    assert!(
        resources.mutating_calls().is_empty(),
        "nothing is sent before the answer"
    );

    app.press("enter");
    app.tick();
    app.tick();
    let sent: Vec<(String, serde_json::Value)> = resources
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch { name, patch, .. } => {
                (name, patch.body["metadata"]["annotations"][DEFAULT].clone())
            }
            other => panic!("unexpected write {other:?}"),
        })
        .collect();
    assert_eq!(
        sent,
        [
            ("nginx".to_owned(), json!("true")),
            ("traefik".to_owned(), serde_json::Value::Null)
        ]
    );
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "ingressclass::SetDefault");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(audit[0].target, class_ref("nginx"));
}

#[gpui::test]
fn a_read_only_cluster_refuses_it_whoever_asks(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let table = app.open_ingress_classes();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("the session is open");

    // The table says why and asks nothing.
    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            let targets = table.action_targets(cx);
            table.run_action(CommandId::INGRESS_CLASS_SET_DEFAULT, targets, window, cx);
        })
    });
    app.tick();
    app.expect_toast("This cluster is read-only");
    assert!(app.class_confirmation().is_none());

    // And the guard refuses every other way in.
    let bus = state.command_bus().expect("the bus");
    let command = Command::IngressClassSetDefault {
        target: class_ref("nginx"),
        clear_previous: true,
    };
    for initiator in [Initiator::Agent, Initiator::Command, Initiator::Ui] {
        let outcome = futures::executor::block_on(
            bus.dispatch(command.clone(), DispatchContext::new(initiator, "someone")),
        );
        assert!(
            matches!(outcome, Err(oxikube_app::DispatchError::ReadOnly { .. })),
            "{initiator}: {outcome:?}"
        );
    }
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    assert!(resources.mutating_calls().is_empty());
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 3);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[gpui::test]
fn the_sidebar_has_no_gateway_api_entry_on_a_cluster_that_does_not_serve_it(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([
        kind("", "Service", "services", true),
        kind("networking.k8s.io", "Ingress", "ingresses", true),
    ]);
    app.press("enter");
    app.tick();
    assert!(app.drawn("sidebar-entry-network/ingresses"));
    assert!(!app.drawn("sidebar-entry-network/gateways"));
    assert!(!app.drawn("sidebar-entry-network/httproutes"));
}

#[gpui::test]
fn the_sidebar_lists_the_gateway_api_kinds_under_network_where_discovery_serves_them(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([
        kind("networking.k8s.io", "Ingress", "ingresses", true),
        kind("gateway.networking.k8s.io", "Gateway", "gateways", true),
        kind("gateway.networking.k8s.io", "HTTPRoute", "httproutes", true),
    ]);
    app.press("enter");
    app.tick();
    assert!(app.drawn("sidebar-entry-network/gateways"));
    assert!(app.drawn("sidebar-entry-network/httproutes"));
    assert!(
        !app.drawn("sidebar-entry-network/gatewayclasses"),
        "a kind the cluster does not serve is not listed"
    );
}
