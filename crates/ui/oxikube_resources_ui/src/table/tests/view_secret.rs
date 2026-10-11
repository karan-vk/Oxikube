//! A Secrets table (E12-S11): `x` reveals the cursor row's Secret, `shift-e` edits it decoded, and
//! "New" creates a Secret instead of picking a template. The keys are bound to the Secrets kind
//! only.

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use serde_json::json;

use super::fixture::{Fixture, cluster};
use super::p;
use crate::actions::tests::toasts;
use crate::detail::tests::fixture::kind;
use crate::kinds;

fn secrets_kind() -> oxikube_domain::kinds::ResourceKind {
    kind("", "v1", "Secret", "secrets", true)
}

fn creds() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "creds", "namespace": "shop", "resourceVersion": "5"},
        "type": "Opaque", "data": {"password": "c3VwZXItc2VjcmV0"}
    }))
    .unwrap()
}

fn target() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Secret"), "shop", "creds")
}

fn secrets_table(cx: &mut TestAppContext) -> (Fixture, gpui::Entity<crate::table::ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    f.vcx.update(|_, cx| kinds::secret::init(cx));
    f.ports().discovery.set_kinds([secrets_kind()]);
    f.ports().resources.insert(creds());
    f.connect_with([]);
    let table = f.open(secrets_kind());
    f.keys(&table, "down");
    f.dispatcher.clear();
    (f, table)
}

#[gpui::test]
fn x_reveals_the_cursor_secret(cx: &mut TestAppContext) {
    let (mut f, table) = secrets_table(cx);
    f.keys(&table, "x");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::SecretReveal {
            target: target(),
            key: None
        }]
    );
}

#[gpui::test]
fn shift_e_asks_for_the_decoded_editor(cx: &mut TestAppContext) {
    let (mut f, table) = secrets_table(cx);
    f.keys(&table, "shift-e");
    // `secret::EditDecoded` is the editor crate's command: this fixture's bus does not have it,
    // so the key says so, as `e` does without the editor.
    let messages: Vec<String> = toasts(&mut f)
        .into_iter()
        .map(|t| t.message.to_string())
        .collect();
    assert_eq!(messages, ["Edit decoded is available for Secrets"]);
}

#[gpui::test]
fn x_is_bound_to_secrets_only(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "apple-1", "1")]);
    let table = f.open_pods();
    f.keys(&table, "down");
    f.dispatcher.clear();
    f.keys(&table, "x");
    assert!(f.dispatcher.sent().is_empty());
    // The binding is scoped to `kind == Secret`: the key is free on other kinds, and says nothing.
    assert!(toasts(&mut f).is_empty());
}

#[gpui::test]
fn new_on_the_secrets_table_creates_a_secret(cx: &mut TestAppContext) {
    let (mut f, _table) = secrets_table(cx);
    f.vcx.run_until_parked();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = f
        .vcx
        .debug_bounds("resource-table-create")
        .expect("the New button");
    f.vcx.simulate_click(bounds.center(), Default::default());
    f.settle();
    assert_eq!(
        f.dispatcher.sent(),
        [Command::SecretCreate {
            cluster: Some(cluster())
        }]
    );
}

#[gpui::test]
fn new_on_the_pods_table_still_opens_the_template_picker(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.vcx.update(|_, cx| kinds::secret::init(cx));
    f.connect_with([p("x", "apple-1", "1")]);
    let _table = f.open_pods();
    f.dispatcher.clear();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = f
        .vcx
        .debug_bounds("resource-table-create")
        .expect("the New button");
    f.vcx.simulate_click(bounds.center(), Default::default());
    f.settle();
    assert_eq!(
        f.dispatcher.sent(),
        [Command::ResourceCreate {
            cluster: Some(cluster()),
            template: None
        }]
    );
}
