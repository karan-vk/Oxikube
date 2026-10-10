//! Scenario B: `:` opens the jump bar from the table, a line is typed and run with Enter, and the
//! app navigates (E11 done-when 2): the Pods table scoped to a namespace, a kind by alias, a label
//! selector forwarded to the feed, a custom resource by its short name, an unknown alias refused.

use gpui::TestAppContext;
use oxikube_domain::ids::Gvk;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{ResourceCall, TestPorts};

use super::{DEFAULT_PODS, SYSTEM_PODS, check};
use crate::mount::tests::App;

impl App {
    /// Opens the bar with `:`, types `line` and presses Enter.
    pub(super) fn colon(&mut self, line: &str) {
        self.press(":");
        check!(
            self,
            self.jump_bar_open().is_some(),
            "`:` opens the jump bar"
        );
        // The contexts and the namespaces are read in the background when the bar opens.
        self.tick();
        self.type_text(line);
        self.press("enter");
        self.tick();
        self.tick();
    }

    /// The calls the cluster's resource port saw for `kind`, in order.
    fn feed_calls(&mut self, kind: &Gvk) -> Vec<ResourceCall> {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
            .recorded_calls()
            .into_iter()
            .filter(|call| match call {
                ResourceCall::List { kind: k, .. } | ResourceCall::Watch { kind: k, .. } => {
                    k == kind
                }
                _ => false,
            })
            .collect()
    }
}

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

#[gpui::test]
fn pods_kube_system_opens_the_table_with_that_namespaces_rows_only(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    let start = app.shown_rows();
    check!(
        app,
        start == DEFAULT_PODS,
        "the table starts on the context's namespace, `default`: {start:?}"
    );

    app.colon("pods kube-system");

    check!(
        app,
        app.jump_bar_open().is_none(),
        "a good line closes the bar"
    );
    let shown = app.shown();
    check!(
        app,
        shown == Some(("Pod".into(), SYSTEM_PODS.map(String::from).to_vec())),
        "only kube-system's pods are shown, got {shown:?}"
    );
    let scoped = app.feed_calls(&pod_gvk()).into_iter().any(|call| {
        matches!(&call, ResourceCall::Watch { namespace: Some(ns), .. }
            | ResourceCall::List { namespace: Some(ns), .. } if ns == "kube-system")
    });
    check!(
        app,
        scoped,
        "the feed was asked for kube-system: {:?}",
        app.feed_calls(&pod_gvk())
    );
    assert!(
        app.ports.state.audit_log().is_empty(),
        "navigation mutates nothing"
    );
}

#[gpui::test]
fn deploy_opens_the_deployments_table_by_its_short_name(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    app.colon("deploy");
    let shown = app.shown();
    check!(
        app,
        shown == Some(("Deployment".into(), vec!["api".into()])),
        "the Deployments of the current namespace, got {shown:?}"
    );
    // With a namespace in the line, that namespace's.
    app.focus_table();
    app.colon("deploy kube-system");
    let shown = app.shown();
    check!(
        app,
        shown == Some(("Deployment".into(), vec!["coredns".into()])),
        "got {shown:?}"
    );
}

#[gpui::test]
fn pod_with_a_selector_forwards_it_to_the_feed_and_narrows_the_rows(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    app.colon("pod app=nginx");
    check!(
        app,
        app.shown_rows() == ["nginx-a", "nginx-b"],
        "only the labelled pods are shown"
    );
    let selectors: Vec<Option<String>> = app
        .feed_calls(&pod_gvk())
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Watch { options, .. } => Some(options.label_selector),
            ResourceCall::List { options, .. } => Some(options.label_selector),
            _ => None,
        })
        .collect();
    check!(
        app,
        selectors.iter().any(|s| s.as_deref() == Some("app=nginx")),
        "the selector reached the feed (applied by the server): {selectors:?}"
    );
}

#[gpui::test]
fn a_custom_resource_opens_by_its_discovered_short_name(cx: &mut TestAppContext) {
    use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};
    use oxikube_testkit::Timeline;

    let mut app = App::keyboard(cx, false);
    // A custom resource's table is the server's own Table: one column, one row.
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    ports.tables.script().table_feed.push_ok(
        Timeline::immediate([TableBatch {
            columns: Some(
                vec![TableColumn {
                    name: "Name".into(),
                    column_type: "string".into(),
                    ..TableColumn::default()
                }]
                .into(),
            ),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![TableRow {
                cells: vec![serde_json::json!("gizmo")],
                meta: Some(oxikube_domain::ObjectMeta::named("gizmo")),
                object: None,
            }])]),
            source: TableSource::Server,
        }])
        .keep_open(),
    );
    app.colon("wd");
    let shown = app.shown();
    check!(
        app,
        shown == Some(("Widget".into(), vec!["gizmo".into()])),
        "`:wd` is the Widget CRD's short name, got {shown:?}"
    );
    // Its plural works too.
    app.focus_table();
    app.colon("widgets");
    assert_eq!(app.shown().map(|(kind, _)| kind), Some("Widget".into()));
}

#[gpui::test]
fn an_unknown_alias_stays_in_the_bar_as_an_error_and_nothing_navigates(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    let before = app.shown();
    app.colon("podz");
    check!(
        app,
        app.jump_bar_open().is_some(),
        "the bar stays open on a mistake"
    );
    check!(
        app,
        app.drawn("jump-problem"),
        "with the problem shown under the line"
    );
    check!(app, app.shown() == before, "the table did not change");
    let tables = app.tab_workspace();
    let open = app
        .vcx
        .update(|_, cx| tables.read(cx).items_of_type::<ResourceTable>().len());
    assert_eq!(open, 1, "no new table opened");
    // Escape gives the keys back to the table.
    app.press("escape");
    check!(app, app.jump_bar_open().is_none(), "escape closes the bar");
    app.press("down");
    assert!(app.cursor().is_some(), "the table has the focus again");
    assert!(app.ports.state.audit_log().is_empty());
}
