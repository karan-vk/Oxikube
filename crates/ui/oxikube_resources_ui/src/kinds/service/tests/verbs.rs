//! The Service's k9s verbs and row actions (E12-S13), pressed in a real Services table with the
//! shipped keymap: `u` shows the pods, `o` opens in the browser. They go key -> table action ->
//! row-action registry -> `service::ShowPods` -> the controller, which reads the Service and sends
//! the commands the jump line sends; every refusal along the way is a toast.

use gpui::{Entity, TestAppContext};
use oxikube_domain::access::AccessRules;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_testkit::resource;
use serde_json::json;

use super::{service_ref, shop};
use crate::actions::tests::toasts;
use crate::detail::tests::fixture::Detail;
use crate::kinds::tests::sidebar_panel;
use crate::table::ResourceTable;
use crate::table::tests::fixture::cluster;

/// A window over `objects` with the exec rig (the real bus, guard and row-action registry, with
/// the Service row actions), the kind specs and open rights.
fn window(cx: &mut TestAppContext, objects: Vec<oxikube_domain::Resource>) -> Detail {
    let mut d = Detail::with_exec(cx, objects);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d.f.ports().access.set_rules(AccessRules::all_access());
    d.settle();
    d
}

/// Network -> Services, through the sidebar, as a user gets there.
fn open_services(d: &mut Detail) -> Entity<ResourceTable> {
    let panel = sidebar_panel(&mut d.f);
    d.f.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.activate("network/services", cx)));
    d.settle();
    let workspace = d.workspace();
    d.f.vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<ResourceTable>())
        .into_iter()
        .find(|t| d.f.vcx.update(|_, cx| &*t.read(cx).gvk().kind == "Service"))
        .expect("the Services table")
}

fn pod_tables(d: &mut Detail) -> Vec<Entity<ResourceTable>> {
    let workspace = d.workspace();
    d.f.vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<ResourceTable>())
        .into_iter()
        .filter(|t| d.f.vcx.update(|_, cx| t.read(cx).gvk().is_pod()))
        .collect()
}

fn messages(d: &mut Detail) -> Vec<String> {
    toasts(&mut d.f)
        .into_iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

#[gpui::test]
fn the_row_menu_and_the_palette_offer_show_pods_on_services_only(cx: &mut TestAppContext) {
    let mut d = window(cx, shop());
    let table = open_services(&mut d);
    let actions = d.f.deps.actions.clone().expect("the rig has row actions");
    let kind = d.f.vcx.update(|_, cx| table.read(cx).kind().clone());

    let labels: Vec<_> = actions
        .entries(&cluster(), &kind, 1)
        .iter()
        .map(|entry| (entry.command(), entry.action.label().to_owned()))
        .collect();
    assert!(
        labels.contains(&(CommandId::SERVICE_SHOW_PODS, "Show Pods".to_owned())),
        "{labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|(command, _)| *command == CommandId::SERVICE_OPEN_IN_BROWSER),
        "Open in Browser waits for a port-forward manager: {labels:?}"
    );

    let pod_kind = crate::table::tests::pods_kind();
    assert!(
        actions
            .entries(&cluster(), &pod_kind, 1)
            .iter()
            .all(|entry| entry.command() != CommandId::SERVICE_SHOW_PODS),
        "Pods do not get Show Pods"
    );
}

#[gpui::test]
fn u_on_a_service_opens_its_pods_in_the_services_namespace_only(cx: &mut TestAppContext) {
    let mut d = window(cx, shop());
    let table = open_services(&mut d);
    d.f.dispatcher.clear();

    // All namespaces are selected: the pods of other namespaces sharing the labels must not show.
    d.f.keys(&table, "j u");
    d.settle();

    let sent = d.f.dispatcher.sent();
    let gvk = Gvk::new("", "v1", "Pod");
    assert_eq!(
        sent,
        [
            Command::ServiceShowPods {
                target: service_ref("web")
            },
            Command::NamespaceSelect {
                cluster: cluster(),
                namespaces: vec!["shop".to_owned()],
            },
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: gvk.clone(),
            },
            Command::TableSetFilter {
                cluster: cluster(),
                gvk,
                text: "-l app=web".to_owned(),
            },
        ]
    );
    assert_eq!(pod_tables(&mut d).len(), 1, "the Pods table opened");
}

#[gpui::test]
fn u_on_a_service_without_a_selector_says_why_and_opens_nothing(cx: &mut TestAppContext) {
    let manual = resource("v1", "Service")
        .name("manual")
        .namespace("shop")
        .field("spec", json!({"ports": [{"port": 80}]}))
        .build();
    let mut d = window(cx, vec![manual]);
    let table = open_services(&mut d);
    d.f.dispatcher.clear();

    d.f.keys(&table, "j u");
    d.settle();

    let explanation = oxikube_domain::view::NoSelector::Manual.explanation();
    assert!(
        messages(&mut d).iter().any(|m| m == explanation),
        "{:?}",
        messages(&mut d)
    );
    assert!(pod_tables(&mut d).is_empty(), "no Pods table was opened");
    assert!(
        !d.f.dispatcher
            .sent()
            .iter()
            .any(|c| matches!(c, Command::TableSetFilter { .. })),
        "no filter was sent"
    );
}

#[gpui::test]
fn u_on_a_service_that_cannot_be_read_says_so(cx: &mut TestAppContext) {
    let mut d = window(cx, shop());
    let table = open_services(&mut d);
    // The Service goes away between the table's list and the key press.
    let removed =
        d.f.ports()
            .resources
            .remove(&Gvk::new("", "v1", "Service"), Some("shop"), "web");
    assert!(removed);
    d.f.dispatcher.clear();

    d.f.keys(&table, "j u");
    d.settle();

    let said = messages(&mut d);
    assert!(
        said.iter().any(|m| m.starts_with("Could not read web")),
        "{said:?}"
    );
    assert!(pod_tables(&mut d).is_empty());
}

#[gpui::test]
fn show_pods_from_the_palette_after_the_connection_dropped_opens_nothing(cx: &mut TestAppContext) {
    use oxikube_workspace::CommandDispatcher as _;
    let mut d = window(cx, shop());
    open_services(&mut d);
    // The connection drops and the cluster's tab closes with it: there is no reader to ask and
    // no tab to tell (the "not connected" toast needs a tab, which only a racing command finds).
    d.f.sessions.disconnect(&cluster()).ok();
    d.settle();
    d.f.dispatcher.clear();

    // The palette and an agent have the reference and nothing else.
    let dispatcher = d.f.dispatcher.clone();
    d.f.vcx.update(|_, cx| {
        dispatcher.dispatch(
            Command::ServiceShowPods {
                target: service_ref("web"),
            },
            cx,
        );
    });
    d.settle();

    assert_eq!(
        d.f.dispatcher.sent().len(),
        1,
        "only the command itself: no namespace is selected and no table opened for a cluster \
         that cannot be read: {:?}",
        d.f.dispatcher.sent()
    );
}

#[gpui::test]
fn o_says_it_needs_port_forwarding_until_a_manager_installs_the_opener(cx: &mut TestAppContext) {
    let mut d = window(cx, shop());
    let table = open_services(&mut d);
    d.f.keys(&table, "j");
    d.f.dispatcher.clear();

    d.f.keys(&table, "o");

    let said = messages(&mut d);
    assert_eq!(
        said,
        ["Open in Browser is available for Services once port forwarding is installed"]
    );
    assert!(
        d.f.dispatcher.sent().is_empty(),
        "no command was dispatched: {:?}",
        d.f.dispatcher.sent()
    );
}
