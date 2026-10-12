//! `u` on a table of a referenced kind (E12-S18) is `resource::ShowUsedBy` for the cursor row; on
//! any other kind the key is free.

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

fn config_map() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "cfg", "namespace": "shop", "resourceVersion": "5"},
        "data": {"k": "v"}
    }))
    .unwrap()
}

#[gpui::test]
fn u_shows_what_uses_the_cursor_row(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    let configmaps = kind("", "v1", "ConfigMap", "configmaps", true);
    f.vcx.update(|_, cx| kinds::register_config_kinds(cx));
    f.ports().discovery.set_kinds([configmaps.clone()]);
    f.ports().resources.insert(config_map());
    f.connect_with([]);
    let table = f.open(configmaps);
    f.keys(&table, "down");
    f.dispatcher.clear();
    f.keys(&table, "u");
    assert_eq!(
        f.dispatcher.sent(),
        [Command::ResourceShowUsedBy {
            target: ResourceRef::namespaced(
                cluster(),
                Gvk::new("", "v1", "ConfigMap"),
                "shop",
                "cfg"
            )
        }]
    );
}

#[gpui::test]
fn u_is_free_on_the_pods_table(cx: &mut TestAppContext) {
    let mut f = Fixture::with_exec(cx);
    f.connect_with([p("x", "apple-1", "1")]);
    let table = f.open_pods();
    f.keys(&table, "down");
    f.dispatcher.clear();
    f.keys(&table, "u");
    assert!(f.dispatcher.sent().is_empty());
    assert!(toasts(&mut f).is_empty());
}
