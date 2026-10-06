//! Custom resources in the table and the CRD list over testkit fakes: a CRD row opens its custom
//! resource table at the storage version, the version switcher, the basic-columns note when the
//! server ignores the Table `Accept` header, printer columns, and how the namespace selection
//! applies to namespaced and cluster-scoped kinds.

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::TableSource;
use oxikube_testkit::{TableCall, Timeline};
use serde_json::json;

use super::fixture::{
    batch, columns, crd_kind, fleet_kind, widget_crd, widget_crd_json, widget_kind,
};
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

/// Connects `cluster` serving `kinds`, with `objects` in the cluster.
fn connect(f: &mut Fixture, kinds: Vec<ResourceKind>, objects: impl IntoIterator<Item = Resource>) {
    let ports = f.ports();
    ports.discovery.set_kinds(kinds);
    for object in objects {
        ports.resources.insert(object);
    }
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
}

/// Scripts the next Table feed of a custom kind.
fn script(f: &Fixture, batch: oxikube_ports::TableBatch) {
    f.ports()
        .tables
        .script()
        .table_feed
        .push_ok(Timeline::immediate([batch]).keep_open());
}

fn widget_columns() -> std::sync::Arc<[oxikube_ports::TableColumn]> {
    columns(&[
        ("Name", "string", 0),
        ("Size", "string", 0),
        ("Replicas", "integer", 0),
        ("Age", "date", 0),
        ("Owner", "string", 1),
    ])
}

fn widget_rows() -> Vec<(Option<&'static str>, &'static str, Vec<serde_json::Value>)> {
    vec![
        (
            Some("shop"),
            "w-1",
            vec![
                json!("w-1"),
                json!("small"),
                json!(1),
                json!("2d"),
                json!("team-a"),
            ],
        ),
        (
            Some("shop"),
            "w-2",
            vec![
                json!("w-2"),
                json!("large"),
                json!(7),
                json!("1d"),
                json!("team-b"),
            ],
        ),
    ]
}

fn tables_of(f: &mut Fixture, gvk: &Gvk) -> Vec<Entity<ResourceTable>> {
    let views = f.views.clone();
    f.vcx
        .update(|_, cx| views.read(cx).tables(&cluster(), gvk, cx))
}

fn visible(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(|id| id.to_string())
                .collect()
        })
    })
}

fn cell(f: &mut Fixture, table: &Entity<ResourceTable>, row: usize, column: &str) -> String {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let object = d.row(row).expect("a row");
            d.provider()
                .cell(
                    object,
                    &oxikube_app::ColumnId::new(column),
                    jiff::Timestamp::now(),
                )
                .display()
                .to_owned()
        })
    })
}

#[gpui::test]
fn a_custom_resource_table_has_the_servers_printer_columns_and_values(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    connect(&mut f, vec![widget_kind("v1", true)], []);
    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()),
    );
    let table = f.open(widget_kind("v1", true));

    assert_eq!(f.names(&table), ["w-1", "w-2"]);
    // Default columns as `kubectl get`; `Owner` (priority 1) is wide, so hidden until picked.
    let mut shown = visible(&mut f, &table);
    shown.sort();
    assert_eq!(shown, ["age", "name", "replicas", "size"]);
    assert_eq!(cell(&mut f, &table, 0, "size"), "small");
    assert_eq!(cell(&mut f, &table, 1, "replicas"), "7");
    assert_eq!(cell(&mut f, &table, 1, "owner"), "team-b");
    assert!(!table.read_with(&f.vcx, |t, _| t.basic_columns()));
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-table-basic-columns").is_none());
    assert!(
        f.vcx.debug_bounds("resource-table-count").is_some(),
        "the toolbar is drawn"
    );
    // The feed is the Table feed (ADR 0006), not a reflector, for a custom kind.
    let calls = f.ports().tables.recorded_calls();
    assert!(
        matches!(&calls[..], [TableCall::TableFeed { kind, .. }] if kind == &widget_kind("v1", true).gvk),
        "{calls:?}"
    );
}

#[gpui::test]
fn plain_objects_from_an_api_that_ignores_the_table_header_say_so(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    connect(&mut f, vec![widget_kind("v1", true)], []);
    // What the adapter synthesises when the server sends plain objects.
    script(
        &f,
        batch(
            TableSource::Objects,
            columns(&[("Name", "string", 0), ("Created At", "date", 0)]),
            &[(
                Some("shop"),
                "w-1",
                vec![json!("w-1"), json!("2026-01-01T00:00:00Z")],
            )],
        ),
    );
    let table = f.open(widget_kind("v1", true));
    assert_eq!(f.names(&table), ["w-1"]);
    assert!(table.read_with(&f.vcx, |t, _| t.basic_columns()));
    // The generic set (Name, Namespace, Age), not the server's stand-in `Created At`.
    let mut shown = visible(&mut f, &table);
    shown.sort();
    assert_eq!(shown, ["age", "name", "namespace"]);
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        f.vcx.debug_bounds("resource-table-basic-columns").is_some(),
        "the toolbar says the columns are the basic ones"
    );
}

#[gpui::test]
fn a_crd_with_several_served_versions_offers_a_switcher_and_each_version_is_a_tab(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    connect(
        &mut f,
        vec![widget_kind("v1beta1", false), widget_kind("v1", true)],
        [],
    );
    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()),
    );
    let table = f.open(widget_kind("v1", true));

    let versions: Vec<String> = table.read_with(&f.vcx, |t, _| {
        t.served_versions()
            .iter()
            .map(|k| k.gvk.version.to_string())
            .collect()
    });
    assert_eq!(versions, ["v1", "v1beta1"], "newest first");
    assert!(table.read_with(&f.vcx, |t, _| t.has_version_switcher()));
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-table-version").is_some());
    let title = f
        .vcx
        .update(|_, cx| gpui::Focusable::focus_handle(table.read(cx), cx));
    let _ = title;
    let tab_title = table.read_with(&f.vcx, |t, _| t.title().to_string());
    assert_eq!(tab_title, "Widgets (v1)");

    // Choosing the other version sends `resource::OpenList` for it: its own tab.
    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()[..1]),
    );
    f.dispatcher.clear();
    f.update(&table, |t, cx| t.switch_version("v1beta1", cx));
    assert_eq!(
        f.dispatcher.sent(),
        [Command::ResourceOpenList {
            cluster: cluster(),
            gvk: widget_kind("v1beta1", false).gvk,
        }]
    );
    let beta = tables_of(&mut f, &widget_kind("v1beta1", false).gvk);
    assert_eq!(beta.len(), 1, "the v1beta1 table opened");
    assert_eq!(
        tables_of(&mut f, &widget_kind("v1", true).gvk).len(),
        1,
        "v1 stays"
    );
    assert_eq!(f.names(&beta[0]), ["w-1"]);
    // The version shown is a no-op.
    f.dispatcher.clear();
    f.update(&table, |t, cx| t.switch_version("v1", cx));
    assert!(f.dispatcher.sent().is_empty());
}

#[gpui::test]
fn a_kind_with_one_version_has_no_switcher_and_a_builtin_kind_asks_discovery_for_nothing(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    connect(
        &mut f,
        vec![widget_kind("v1", true), crate::table::tests::pods_kind()],
        [],
    );
    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()),
    );
    let table = f.open(widget_kind("v1", true));
    assert!(!table.read_with(&f.vcx, |t, _| t.has_version_switcher()));
    assert_eq!(
        table.read_with(&f.vcx, |t, _| t.title().to_string()),
        "Widgets"
    );
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resource-table-version").is_none());

    let discovers = f
        .ports()
        .discovery
        .recorded_calls()
        .into_iter()
        .filter(|call| matches!(call, oxikube_testkit::DiscoveryCall::Discover))
        .count();
    let pods = f.open_pods();
    let after = f
        .ports()
        .discovery
        .recorded_calls()
        .into_iter()
        .filter(|call| matches!(call, oxikube_testkit::DiscoveryCall::Discover))
        .count();
    assert_eq!(after, discovers, "a built-in kind needs no version lookup");
    assert!(pods.read_with(&f.vcx, |t, _| t.served_versions().is_empty()));
}

#[gpui::test]
fn namespaced_custom_resources_follow_the_namespace_selection_and_cluster_scoped_ones_ignore_it(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    connect(&mut f, vec![widget_kind("v1", true), fleet_kind()], []);
    f.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("shop"))
        .expect("a session");
    f.vcx.run_until_parked();
    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()),
    );
    script(
        &f,
        batch(
            TableSource::Server,
            columns(&[("Name", "string", 0)]),
            &[(None, "fleet-1", vec![json!("fleet-1")])],
        ),
    );
    let widgets = f.open(widget_kind("v1", true));
    let fleets = f.open(fleet_kind());
    assert_eq!(f.names(&widgets), ["w-1", "w-2"]);
    assert_eq!(f.names(&fleets), ["fleet-1"]);

    let feeds: Vec<(String, Option<String>)> = f
        .ports()
        .tables
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            TableCall::TableFeed {
                kind, namespace, ..
            } => Some((kind.kind.to_string(), namespace)),
            TableCall::ListTable { .. } => None,
        })
        .collect();
    assert!(
        feeds.contains(&("Widget".into(), Some("shop".into()))),
        "{feeds:?}"
    );
    assert!(
        feeds.contains(&("Fleet".into(), None)),
        "the cluster-scoped kind is read cluster-wide whatever is selected: {feeds:?}"
    );
}

/// Opens the CRD list over a cluster that has the Widget CRD, with its row in the table.
fn crd_list(f: &mut Fixture) -> Entity<ResourceTable> {
    connect(
        f,
        vec![
            crd_kind(),
            widget_kind("v1beta1", false),
            widget_kind("v1", true),
        ],
        [widget_crd()],
    );
    f.open(crd_kind())
}

#[gpui::test]
fn the_crd_list_shows_group_version_scope_and_short_names(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = crd_list(&mut f);
    assert_eq!(f.names(&table), ["widgets.example.com"]);
    let shown = visible(&mut f, &table);
    assert_eq!(
        shown,
        ["name", "group", "version", "scope", "short-names", "age"]
    );
    assert_eq!(cell(&mut f, &table, 0, "group"), "example.com");
    assert_eq!(
        cell(&mut f, &table, 0, "version"),
        "v1",
        "the storage version"
    );
    assert_eq!(cell(&mut f, &table, 0, "scope"), "Namespaced");
    assert_eq!(cell(&mut f, &table, 0, "short-names"), "wd,wdg");
}

#[gpui::test]
fn opening_a_crd_row_opens_the_table_of_its_custom_resources_at_the_storage_version(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    let table = crd_list(&mut f);
    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()),
    );
    f.dispatcher.clear();

    // Enter / double click on the row: `crd::OpenResources` with the CRD's name.
    f.update(&table, |t, cx| t.open_row(0, cx));
    assert_eq!(
        f.dispatcher.sent()[0],
        Command::CrdOpenResources {
            cluster: cluster(),
            name: "widgets.example.com".into()
        }
    );
    f.settle();
    let opened = tables_of(&mut f, &widget_kind("v1", true).gvk);
    assert_eq!(
        opened.len(),
        1,
        "the Widget table at v1, the storage version"
    );
    assert_eq!(f.names(&opened[0]), ["w-1", "w-2"]);
    assert!(
        tables_of(&mut f, &widget_kind("v1beta1", false).gvk).is_empty(),
        "not the deprecated one"
    );
    // Opening it again focuses the same tab.
    f.update(&table, |t, cx| t.open_row(0, cx));
    f.settle();
    assert_eq!(tables_of(&mut f, &widget_kind("v1", true).gvk).len(), 1);
}

#[gpui::test]
fn a_crd_that_serves_nothing_says_so_instead_of_opening_a_table(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let mut json = widget_crd_json();
    for version in json["spec"]["versions"].as_array_mut().unwrap() {
        version["served"] = json!(false);
    }
    connect(
        &mut f,
        vec![crd_kind()],
        [Resource::from_json(json).unwrap()],
    );
    let table = f.open(crd_kind());
    f.update(&table, |t, cx| t.open_row(0, cx));
    f.settle();
    assert!(tables_of(&mut f, &widget_kind("v1", true).gvk).is_empty());
    // A toast in the cluster's tab says why.
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .unwrap();
    let toasts: Vec<String> = f.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .toast_layer()
            .read(cx)
            .visible()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    });
    assert!(
        toasts.iter().any(|t| t.contains("is not served")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn the_crd_rows_menu_offers_open_and_details_and_other_kinds_do_not(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    let table = crd_list(&mut f);
    f.update(&table, |t, cx| t.select_all(cx));
    let labels: Vec<String> = table.read_with(&f.vcx, |t, cx| {
        t.action_entries(cx).into_iter().map(|e| e.label).collect()
    });
    assert_eq!(labels, ["Open Custom Resources", "Show Details", "Delete"]);

    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()),
    );
    let target = table.read_with(&f.vcx, |t, cx| t.row_ref(0, cx)).unwrap();
    f.dispatcher.clear();
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.run_action(
                oxikube_domain::command::CommandId::CRD_OPEN_RESOURCES,
                vec![target.clone()],
                window,
                cx,
            )
        })
    });
    f.settle();
    assert_eq!(tables_of(&mut f, &widget_kind("v1", true).gvk).len(), 1);
    // "Show Details" is `resource::Open`: the detail drawer, with the Schema tab.
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.run_action(
                oxikube_domain::command::CommandId::RESOURCE_OPEN,
                vec![target.clone()],
                window,
                cx,
            )
        })
    });
    f.settle();
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .any(|c| matches!(c, Command::ResourceOpen { .. }))
    );

    let widgets = tables_of(&mut f, &widget_kind("v1", true).gvk);
    let widget_labels: Vec<String> = widgets[0].read_with(&f.vcx, |t, cx| {
        t.action_entries(cx).into_iter().map(|e| e.label).collect()
    });
    assert_eq!(
        widget_labels,
        ["Delete"],
        "a custom resource row has the generic actions only"
    );
}

/// The sidebar of the cluster's tab.
fn sidebar(f: &mut Fixture) -> Entity<oxikube_workspace::sidebar::SidebarPanel> {
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    f.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .panel::<oxikube_workspace::sidebar::SidebarPanel>()
            .expect("the tab's sidebar")
    })
}

#[gpui::test]
fn the_sidebar_leads_to_the_crd_list_and_to_each_custom_kind(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    connect(
        &mut f,
        vec![crd_kind(), widget_kind("v1", true), fleet_kind()],
        [widget_crd()],
    );
    script(
        &f,
        batch(TableSource::Server, widget_columns(), &widget_rows()),
    );
    let panel = sidebar(&mut f);
    f.settle();

    // The section lists the groups, closed, with how many kinds each has.
    let ids: Vec<String> = f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .rows()
            .iter()
            .map(|r| r.id().to_owned())
            .collect()
    });
    assert!(
        ids.contains(&"custom-resources/definitions".to_owned()),
        "{ids:?}"
    );
    assert!(ids.contains(&"crd:example.com".to_owned()));
    assert!(
        !ids.contains(&"crd:example.com/widgets".to_owned()),
        "collapsed by default"
    );
    let kinds = f
        .vcx
        .update(|_, cx| match panel.read(cx).row("crd:example.com") {
            Some(oxikube_workspace::sidebar::Row::Group(g)) => g.count,
            _ => None,
        });
    assert_eq!(kinds, Some(2), "Widget and Fleet");
    assert_eq!(
        f.ports().tables.live_feeds(),
        0,
        "listing the groups started no feed"
    );

    // "Definitions": `crd::OpenList` on the bus, then the CRD list.
    f.dispatcher.clear();
    f.vcx
        .update(|_, cx| panel.update(cx, |p, cx| p.activate("custom-resources/definitions", cx)));
    f.settle();
    assert_eq!(
        f.dispatcher.sent(),
        [
            Command::CrdOpenList { cluster: cluster() },
            // ... which is the list of the CRD kind.
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: crd_kind().gvk,
            },
        ]
    );
    let crds = tables_of(&mut f, &crd_kind().gvk);
    assert_eq!(crds.len(), 1, "the CRD list opened");
    assert_eq!(f.names(&crds[0]), ["widgets.example.com"]);

    // A group opens with a click and a kind in it opens its table, on the Table feed.
    f.vcx
        .update(|_, cx| panel.update(cx, |p, cx| p.activate("crd:example.com", cx)));
    f.settle();
    f.vcx
        .update(|_, cx| panel.update(cx, |p, cx| p.activate("crd:example.com/widgets", cx)));
    f.settle();
    let widgets = tables_of(&mut f, &widget_kind("v1", true).gvk);
    assert_eq!(widgets.len(), 1);
    assert_eq!(f.names(&widgets[0]), ["w-1", "w-2"]);
}
