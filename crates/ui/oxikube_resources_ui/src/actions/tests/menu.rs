//! The actions the context menu and the palette list, from the bus's registry.

use gpui::TestAppContext;
use oxikube_domain::command::CommandId;

use super::{choose, dialog, nodes_kind, readonly_kind, right_click, toasts};
use crate::table::tests::fixture::Fixture;
use crate::table::tests::p;

fn labels(f: &mut Fixture, table: &gpui::Entity<crate::table::ResourceTable>) -> Vec<String> {
    f.vcx
        .update(|_, cx| table.read(cx).action_entries(cx))
        .into_iter()
        .map(|entry| entry.label)
        .collect()
}

#[gpui::test]
fn a_pod_and_another_kind_get_the_actions_the_registry_has_for_them(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let pods = f.open_pods();
    // Pods have the per-kind action; delete is for every kind the server can delete.
    assert_eq!(labels(&mut f, &pods), ["Logs", "Delete"]);

    let nodes = f.open(nodes_kind());
    assert_eq!(
        labels(&mut f, &nodes),
        ["Delete"],
        "no pod-only action on a node"
    );

    let status = f.open(readonly_kind());
    assert!(
        labels(&mut f, &status).is_empty(),
        "a kind the server cannot delete has no delete"
    );
}

#[gpui::test]
fn the_palette_list_is_the_menu_list_with_the_same_state(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "web-1", "1")]);
    let pods = f.open_pods();
    f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .unwrap();
    let entries = f.vcx.update(|_, cx| pods.read(cx).action_entries(cx));
    let delete = entries
        .iter()
        .find(|e| e.command() == CommandId::RESOURCE_DELETE)
        .expect("delete is listed, disabled");
    assert!(!delete.is_enabled());
    assert_eq!(
        delete.reason().as_deref(),
        Some("This cluster is read-only")
    );
    let logs = entries
        .iter()
        .find(|e| e.command() == CommandId::POD_VIEW_LOGS)
        .unwrap();
    assert!(logs.is_enabled(), "a read stays available");
}

#[gpui::test]
fn the_context_menu_item_opens_the_delete_dialog(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "web-1", "1")]);
    let table = f.open_pods();
    assert_eq!(f.names(&table), ["web-0", "web-1"]);
    right_click(&mut f, 1);
    // Open, Copy Name, Select All, Logs, Delete.
    choose(&mut f, 4);
    let dialog = dialog(&mut f).expect("the delete dialog opened");
    let (items, tier) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (
            d.plan()
                .items()
                .iter()
                .map(|i| i.target.name.to_string())
                .collect::<Vec<_>>(),
            d.plan().tier(),
        )
    });
    assert_eq!(items, ["web-1"], "the object right-clicked");
    assert_eq!(tier, oxikube_domain::safety::ConfirmTier::Simple);
    assert!(
        f.ports().resources.mutating_calls().is_empty(),
        "nothing is sent by opening it"
    );
}

#[gpui::test]
fn the_per_kind_action_dispatches_its_command(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let _table = f.open_pods();
    f.dispatcher.clear();
    right_click(&mut f, 0);
    choose(&mut f, 3); // Logs
    let sent = f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [oxikube_domain::command::Command::PodViewLogs { target, .. }] if &*target.name == "web-0"),
        "{sent:?}"
    );
    assert!(dialog(&mut f).is_none(), "a read has no dialog");
}

#[gpui::test]
fn a_disabled_delete_does_not_open_a_dialog_and_says_why(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .unwrap();
    right_click(&mut f, 0);
    // Whichever enabled item the keys land on, never Delete: the menu does not select a
    // disabled item (asserted on the items in `a_disabled_action_is_a_disabled_item_with_its_reason`).
    choose(&mut f, 4);
    assert!(dialog(&mut f).is_none());
    assert!(f.ports().resources.mutating_calls().is_empty());

    // The delete key goes the same way: no dialog, a toast with the reason.
    f.keys(&table, "delete");
    assert!(dialog(&mut f).is_none());
    let toasts = toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
    assert!(f.state.audit_log().is_empty(), "nothing reached the guard");
}

#[gpui::test]
fn a_disabled_action_is_a_disabled_item_with_its_reason(cx: &mut TestAppContext) {
    use oxikube_ui::menu::PopupMenuItem;

    let mut f = Fixture::with_actions(cx);
    f.connect_with([p("x", "web-0", "1")]);
    let table = f.open_pods();
    let items_of = |f: &mut Fixture, command: CommandId| {
        let view = table.downgrade();
        f.vcx.update(|_, cx| {
            let entry = table
                .read(cx)
                .action_entries(cx)
                .into_iter()
                .find(|e| e.command() == command)
                .unwrap();
            crate::actions::menu::entry_items(&entry, &[], &view)
        })
    };

    // Enabled: one clickable item, no reason line.
    let items = items_of(&mut f, CommandId::RESOURCE_DELETE);
    assert!(
        matches!(
            items.as_slice(),
            [PopupMenuItem::Item {
                disabled: false,
                ..
            }]
        ),
        "an enabled delete is one live item"
    );

    // Read-only: the item is disabled (the menu skips it and ignores a click) and the reason
    // follows as a label line.
    f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .unwrap();
    let items = items_of(&mut f, CommandId::RESOURCE_DELETE);
    match items.as_slice() {
        [
            PopupMenuItem::Item {
                disabled: true,
                label,
                ..
            },
            PopupMenuItem::Label(reason),
        ] => {
            assert_eq!(&**label, "Delete");
            assert!(reason.contains("read-only"), "{reason}");
        }
        other => panic!(
            "expected a disabled item and its reason, got {} items",
            other.len()
        ),
    }
}
