//! The k9s verbs of a table (E11-S07), pressed in a real table with the shipped keymap: `y` YAML,
//! `d` describe, `e` edit, `l` logs, `shift-f` / `f` port forwards, `ctrl-w` wide columns. Each
//! dispatches the command the palette and an agent run; where the feature behind it is not
//! installed yet, the key says so instead of doing nothing. With the filter field focused the
//! same letters are text.

use gpui::{KeyContext, TestAppContext};
use oxikube_app::columns::ColumnId;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::StatePort as _;

use super::fixture::{Fixture, cluster};
use super::p;
use crate::actions::tests::{nodes_kind, toasts};
use crate::detail::DetailTab;
use crate::detail::tests::fixture::{Detail, pod_ref, web_pod};
use crate::table::{ColumnPrefs, ResourceTable, prefs_key};

fn apple() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "x", "apple-1")
}

/// The pods table with the actions and its first row on the cursor.
fn open(cx: &mut TestAppContext) -> (Fixture, gpui::Entity<ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "apple-1", "1"), p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    assert_eq!(f.selected(&table), ["apple-1"]);
    f.dispatcher.clear();
    (f, table)
}

fn messages(f: &mut Fixture) -> Vec<String> {
    toasts(f)
        .into_iter()
        .map(|toast| toast.message.to_string())
        .collect()
}

#[gpui::test]
fn y_dispatches_view_yaml_for_the_cursor_row(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "y");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::ResourceViewYaml { target: apple() }]
    );
}

#[gpui::test]
fn d_dispatches_view_describe_for_the_cursor_row(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "d");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::ResourceViewDescribe { target: apple() }]
    );
}

#[gpui::test]
fn y_and_d_open_the_drawer_on_the_yaml_and_the_describe_tab(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let table = d.f.open_pods();
    d.f.keys(&table, "j");
    assert!(d.drawer().is_none(), "nothing is open yet");

    d.f.keys(&table, "y");
    d.settle();
    let view = d.drawer_view().expect("y opened the drawer");
    assert_eq!(d.read(&view, |v| v.target().clone()), pod_ref("web-0"));
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);

    // The same view, on the other tab: the keys are on the table again after the drawer opened
    // (the table has the focus; the drawer gets it on Enter).
    d.f.keys(&table, "d");
    d.settle();
    let view = d.drawer_view().expect("still open");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Describe);
    d.f.keys(&table, "y");
    d.settle();
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
}

#[gpui::test]
fn l_opens_the_logs_of_a_pod(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "l");
    let sent = f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::PodViewLogs { target, .. }] if *target == apple()),
        "{sent:?}"
    );
    assert_eq!(messages(&mut f), [] as [&str; 0]);
}

#[gpui::test]
fn l_on_a_kind_without_logs_says_which_have_them(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.ports().discovery.set_kinds([nodes_kind()]);
    f.ports()
        .resources
        .insert(oxikube_testkit::node().name("worker-1").build());
    f.connect_with([]);
    let table = f.open(nodes_kind());
    f.keys(&table, "down");
    f.keys(&table, "l");
    assert_eq!(
        messages(&mut f),
        ["Logs are available for Pods, workloads and Services"]
    );
    assert!(f.dispatcher.sent().is_empty());
}

const PORT_FORWARDS: &str = "Port forwarding is not available yet";

/// Takes the toast with `key` off the screen (the layer keeps toasts until they expire).
fn dismiss_toast(f: &mut Fixture, key: &str) {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs
            .read(cx)
            .tab(&cluster())
            .cloned()
            .expect("the cluster tab");
        let layer = tab.read(cx).workspace().read(cx).toast_layer().clone();
        let gone = layer.update(cx, |layer, cx| layer.dismiss_key(key, cx));
        assert!(gone, "the toast was showing");
    });
}

#[gpui::test]
fn e_and_the_port_forward_keys_say_when_their_feature_is_not_installed(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "e");
    assert_eq!(messages(&mut f), ["Editing is not available yet"]);
    f.keys(&table, "shift-f");
    assert_eq!(
        messages(&mut f),
        ["Editing is not available yet", PORT_FORWARDS]
    );

    // `f` is checked on its own: with the `shift-f` toast gone, only `f` can bring it back.
    dismiss_toast(
        &mut f,
        &format!("action-unavailable:{}", CommandId::POD_PORT_FORWARD),
    );
    assert_eq!(messages(&mut f), ["Editing is not available yet"]);
    f.keys(&table, "f");
    assert_eq!(
        messages(&mut f),
        ["Editing is not available yet", PORT_FORWARDS]
    );
    assert!(f.dispatcher.sent().is_empty(), "nothing ran");
}

#[gpui::test]
fn row_verbs_act_on_the_cursor_row_of_a_selection_and_say_so(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "shift-j");
    assert_eq!(f.selected(&table), ["apple-1", "web-1"]);
    f.keys(&table, "l");
    let sent = f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::PodViewLogs { target, .. }] if &*target.name == "web-1"),
        "{sent:?}"
    );
    assert!(
        messages(&mut f)
            .iter()
            .any(|m| m.contains("acts on the cursor row (web-1), not the 2 selected")),
        "{:?}",
        messages(&mut f)
    );
}

fn qos_shown(f: &mut Fixture, table: &gpui::Entity<ResourceTable>) -> bool {
    f.vcx.update(|_, cx| {
        table
            .read(cx)
            .read_rows(cx, |d| d.layout.is_shown(&ColumnId::new("qos")))
    })
}

#[gpui::test]
fn ctrl_w_shows_the_wide_columns_and_a_second_press_hides_them(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    assert!(!qos_shown(&mut f, &table), "wide columns start hidden");

    f.keys(&table, "ctrl-w");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::TableToggleWide {
            cluster: cluster(),
            gvk: Gvk::new("", "v1", "Pod"),
        }]
    );
    assert!(qos_shown(&mut f, &table), "the command reached the table");

    f.keys(&table, "ctrl-w");
    assert!(!qos_shown(&mut f, &table), "and back");
}

#[gpui::test]
fn the_wide_toggle_is_saved_with_the_layout(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "ctrl-w");
    f.settle();
    let key = prefs_key(&Gvk::new("", "v1", "Pod")).unwrap();
    let live = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout.prefs()));
    assert_eq!(live.visible.get("qos"), Some(&true), "{live:?}");
    // And the copy in the state store, which is what the next launch reads.
    let saved: ColumnPrefs = futures::executor::block_on(f.state.kv_get(&key))
        .unwrap()
        .map(|v| serde_json::from_value(v).unwrap())
        .expect("the wide toggle was saved");
    assert_eq!(saved.visible.get("qos"), Some(&true), "{saved:?}");
}

#[gpui::test]
fn the_verbs_are_letters_typed_into_the_filter_while_it_has_the_focus(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "/ y d e l f");
    let text = f
        .vcx
        .update(|_, cx| table.read(cx).filter().read(cx).text().to_owned());
    assert_eq!(text, "ydelf");
    let verbs: Vec<_> = f
        .dispatcher
        .sent()
        .into_iter()
        .filter(|c| !matches!(c, Command::TableFocusFilter { .. }))
        .collect();
    assert_eq!(verbs, [], "no verb ran");
    assert_eq!(messages(&mut f), [] as [&str; 0], "and none complained");
}

/// The table's entry in the key context stack of the focused element.
fn table_context(f: &mut Fixture) -> KeyContext {
    f.vcx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window
            .context_stack()
            .into_iter()
            .find(|context| context.contains("ResourceTable"))
            .expect("the focus is inside the table")
    })
}

#[gpui::test]
fn the_table_context_names_its_kind_and_scope(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    f.keys(&table, "j");
    let context = table_context(&mut f);
    assert_eq!(context.get("kind").map(|v| v.as_ref()), Some("Pod"));
    assert_eq!(context.get("scope").map(|v| v.as_ref()), Some("namespaced"));
    assert_eq!(context.get("selection").map(|v| v.as_ref()), Some("one"));
}

#[gpui::test]
fn a_user_section_can_bind_a_key_for_one_kind(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    // E12 binds `s` to scale on Deployments with a section like this; here `s` becomes "wide"
    // for Pods only, and the table's default `s` (shell) is untouched for other kinds.
    f.vcx.update(|_, cx| {
        oxikube_keymap::reload_user_keymap(
            cx,
            r#"[{"context": "ResourceTable && !Editing && kind == Pod",
                 "bindings": {"s": "resource_table::ToggleWide"}}]"#,
        )
    });
    f.keys(&table, "s");
    assert!(
        f.dispatcher
            .sent()
            .iter()
            .any(|c| c.id() == CommandId::TABLE_TOGGLE_WIDE),
        "{:?}",
        f.dispatcher.sent()
    );
}
