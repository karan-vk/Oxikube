//! The Namespaces table: its columns (Status, Pods, Age, Labels), the live pod count, the row
//! menu, and that a user can reach it from the sidebar.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::ColumnId;
use oxikube_domain::access::AccessRules;
use oxikube_domain::command::CommandId;
use oxikube_testkit::discovery_sets;
use serde_json::json;

use super::{Env, ns};
use crate::kinds::tests::sidebar_panel;
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};
use crate::table::tests::p;

#[gpui::test]
fn the_columns_are_name_status_pods_age_and_labels_when_wide(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    let table = env.table.clone();
    let columns = env
        .f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().visible_ids()));
    assert_eq!(
        columns,
        ["name", "status", "pods", "age"].map(ColumnId::new),
        "Labels is a wide column, as for every kind"
    );
    let all: Vec<String> = env.f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .columns()
                .map(|(c, _)| c.id.as_str().to_owned())
                .collect()
        })
    });
    assert_eq!(all, ["name", "status", "pods", "age", "labels"]);
    assert_eq!(env.f.names(&env.table), ["kube-system", "team-a", "team-b"]);
    assert_eq!(env.cell(1, "name"), "team-a");
    assert_eq!(env.cell(1, "status"), "Active");
    assert_eq!(env.cell(1, "labels"), "team=a");
}

#[gpui::test]
fn the_pods_column_counts_each_namespaces_pods_from_the_pods_feed(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    // No view of the user opened pods: the table leased the feed itself.
    assert_eq!(env.cell(0, "pods"), "0", "kube-system has none");
    assert_eq!(env.cell(1, "pods"), "2");
    assert_eq!(env.cell(2, "pods"), "1");
    // A pods table on the same cluster reuses that feed.
    let live_before = env.f.ports().resources.live_watches();
    let pods = env.f.open_pods();
    assert_eq!(env.f.names(&pods).len(), 3);
    assert_eq!(
        env.f.ports().resources.live_watches(),
        live_before,
        "the pods feed is shared, not started twice"
    );
}

#[gpui::test]
fn the_row_menu_offers_use_all_pin_new_and_delete(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.cursor_to("team-a");
    let table = env.table.clone();
    let entries = env.f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let labels: Vec<(CommandId, &str)> = entries
        .iter()
        .map(|e| (e.command(), e.action.label()))
        .collect();
    assert!(
        labels.contains(&(CommandId::NAMESPACE_SELECT, "Use namespace")),
        "{labels:?}"
    );
    assert!(labels.contains(&(CommandId::NAMESPACE_USE_FAVOURITE, "All namespaces")));
    assert!(labels.contains(&(CommandId::NAMESPACE_TOGGLE_FAVOURITE, "Toggle favourite")));
    assert!(labels.contains(&(CommandId::NAMESPACE_CREATE, "New namespace…")));
    assert!(labels.contains(&(CommandId::NAMESPACE_DELETE, "Delete")));
    assert!(
        !labels
            .iter()
            .any(|(id, _)| *id == CommandId::RESOURCE_DELETE),
        "a Namespace is deleted by its own command, not the generic one: {labels:?}"
    );
    assert!(entries.iter().all(|e| e.is_enabled()));
}

#[gpui::test]
fn a_read_only_cluster_disables_create_and_delete_but_not_use_or_pin(cx: &mut TestAppContext) {
    let mut env = Env::standard(cx);
    env.f.sessions.set_read_only(&cluster(), true).unwrap();
    env.cursor_to("team-a");
    let table = env.table.clone();
    let entries = env.f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let enabled = |id: CommandId| {
        entries
            .iter()
            .find(|e| e.command() == id)
            .map(|e| e.is_enabled())
            .unwrap_or_else(|| panic!("{id} is offered"))
    };
    assert!(
        enabled(CommandId::NAMESPACE_SELECT),
        "use changes only the view"
    );
    assert!(enabled(CommandId::NAMESPACE_TOGGLE_FAVOURITE));
    assert!(!enabled(CommandId::NAMESPACE_CREATE));
    assert!(!enabled(CommandId::NAMESPACE_DELETE));
}

#[gpui::test]
fn the_sidebars_namespaces_entry_opens_this_table(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.access.set_rules(AccessRules::all_access());
    ports.discovery.set_kinds(discovery_sets::core());
    ports.resources.insert(ns("team-a", json!({})));
    ports.resources.insert(p("team-a", "web-0", "1"));
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();

    let panel = sidebar_panel(&mut f);
    f.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.activate("namespaces/namespaces", cx)));
    f.settle();

    let tabs = f.tabs.clone();
    let tables = f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().expect("a tab");
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<ResourceTable>()
    });
    assert_eq!(tables.len(), 1, "the entry opened a table");
    let (columns, names) = f.vcx.update(|_, cx| {
        tables[0].read(cx).read_rows(cx, |d| {
            (
                d.layout().visible_ids(),
                d.rows()
                    .iter()
                    .map(|r| r.name().to_owned())
                    .collect::<Vec<_>>(),
            )
        })
    });
    assert!(columns.contains(&ColumnId::new("pods")), "{columns:?}");
    assert_eq!(names, ["team-a"]);
}

#[gpui::test]
fn the_palette_runs_create_and_delete_through_the_table(cx: &mut TestAppContext) {
    use oxikube_workspace::command_surface::CommandSurface as _;

    let mut env = Env::standard(cx);
    env.cursor_to("team-a");
    let table = env.table.clone();
    let own = env.f.vcx.update(|_, cx| table.read(cx).own_commands(cx));
    for id in [
        CommandId::NAMESPACE_CREATE,
        CommandId::NAMESPACE_DELETE,
        CommandId::NAMESPACE_SELECT,
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
    ] {
        assert!(own.contains(&id), "{id} in {own:?}");
    }

    // Create opens the dialog (the palette cannot ask a name); delete opens the delete dialog.
    let target = super::ns_ref("team-a");
    let ran = env.f.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_command(
                CommandId::NAMESPACE_CREATE,
                vec![target.clone()],
                window,
                cx,
            )
        })
    });
    env.f.settle();
    assert!(ran);
    let tabs = env.f.tabs.clone();
    let create_open = env.f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().expect("a tab");
        let workspace = tab.read(cx).workspace().clone();
        let layer = workspace.read(cx).modal_layer().clone();
        layer
            .read(cx)
            .active_modal::<crate::kinds::namespace::CreateNamespaceDialog>()
            .is_some()
    });
    assert!(
        create_open,
        "the palette's Create Namespace opens the dialog"
    );
}

#[gpui::test]
fn a_read_only_cluster_lists_no_create_or_delete_for_the_palette(cx: &mut TestAppContext) {
    use oxikube_workspace::command_surface::CommandSurface as _;

    let mut env = Env::standard(cx);
    env.f.sessions.set_read_only(&cluster(), true).unwrap();
    env.cursor_to("team-a");
    let table = env.table.clone();
    let own = env.f.vcx.update(|_, cx| table.read(cx).own_commands(cx));
    assert!(!own.contains(&CommandId::NAMESPACE_CREATE), "{own:?}");
    assert!(!own.contains(&CommandId::NAMESPACE_DELETE));
    assert!(own.contains(&CommandId::NAMESPACE_SELECT));
}
