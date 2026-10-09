//! The Schema tab of a CRD's detail (E07-S07): the tab is there for a CRD only, shows the schema
//! of the storage version as a collapsible tree, switches version, opens and closes nodes with a
//! click, builds lazily, and leads to the custom resources.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;
use serde_json::json;

use super::fixture::{Detail, pod_ref, web_pod};
use crate::crds::tests::fixture::{fleet_crd_json, widget_crd};
use crate::crds::{RowKind, crd_gvk};
use crate::detail::DetailTab;
use crate::table::tests::fixture::cluster;

fn crd_ref(name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(cluster(), crd_gvk(), name)
}

fn row_keys(d: &mut Detail, view: &gpui::Entity<crate::detail::DetailView>) -> Vec<String> {
    d.read(view, |v| {
        v.schema_rows().iter().map(|r| r.key.to_string()).collect()
    })
}

#[gpui::test]
fn only_a_crd_has_a_schema_tab_and_it_shows_the_storage_versions_fields(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [widget_crd(), web_pod()]);
    let pod = d.open(&pod_ref("web-0"));
    d.draw();
    assert!(d.shown("detail-tab-events"));
    assert!(!d.shown("detail-tab-schema"), "a pod has no schema tab");
    assert!(d.read(&pod, |v| v.crd_info().is_none()));

    let view = d.open(&crd_ref("widgets.example.com"));
    assert!(d.shown("detail-tab-schema"));
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Overview);
    d.click("detail-tab-schema");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Schema);
    assert_eq!(
        d.read(&view, |v| v.schema_version().map(str::to_owned)),
        Some("v1".into())
    );
    assert_eq!(
        d.read(&view, |v| v.schema_versions().to_vec()),
        ["v1alpha1", "v1beta1", "v1"]
    );
    // The top-level fields, `spec` first (it is required), all closed.
    assert_eq!(
        row_keys(&mut d, &view),
        ["spec", "apiVersion", "kind", "metadata", "status"]
    );
    assert!(d.shown("detail-schema-tree"));
    assert!(d.shown("schema-row-spec"));
    assert!(
        !d.shown("schema-row-spec.size"),
        "nothing below is built yet"
    );
    // The summary says what the CRD is.
    let info = d.read(&view, |v| v.crd_info().cloned()).expect("parsed");
    assert_eq!(info.short_names, ["wd", "wdg"]);
    assert!(d.shown("detail-schema-facts"));
    assert!(d.shown("detail-schema-version-v1beta1"));
}

#[gpui::test]
fn a_click_opens_and_closes_a_node_and_a_row_says_type_required_and_enum(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [widget_crd()]);
    let view = d.open(&crd_ref("widgets.example.com"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    d.click("schema-row-spec");
    let keys = row_keys(&mut d, &view);
    assert!(keys.contains(&"spec.size".to_owned()), "{keys:?}");
    assert!(d.shown("schema-row-spec.size"));
    let size = d
        .read(&view, |v| {
            v.schema_rows()
                .iter()
                .find(|r| &*r.key == "spec.size")
                .cloned()
        })
        .unwrap();
    assert!(size.required);
    assert_eq!(size.enum_values, ["small", "medium", "large"]);
    assert_eq!(size.ty, "string");

    // Open a nested array of objects, then close its parent: the subtree goes.
    d.click("schema-row-spec.containers");
    assert!(row_keys(&mut d, &view).contains(&"spec.containers.image".to_owned()));
    d.click("schema-row-spec");
    assert_eq!(
        row_keys(&mut d, &view),
        ["spec", "apiVersion", "kind", "metadata", "status"]
    );
    assert!(!d.shown("schema-row-spec.size"));
    // A row with nothing below does nothing.
    d.click("schema-row-kind");
    assert_eq!(row_keys(&mut d, &view).len(), 5);
    assert!(!d.update(&view, |v, cx| v.toggle_schema("kind", cx)));
}

#[gpui::test]
fn a_version_chip_shows_that_versions_schema(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [widget_crd()]);
    let view = d.open(&crd_ref("widgets.example.com"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    d.click("detail-schema-version-v1beta1");
    assert_eq!(
        d.read(&view, |v| v.schema_version().map(str::to_owned)),
        Some("v1beta1".into())
    );
    assert_eq!(row_keys(&mut d, &view), ["spec"], "v1beta1 has just spec");
    d.click("schema-row-spec");
    assert_eq!(row_keys(&mut d, &view), ["spec", "spec.size"]);
    d.update(&view, |v, cx| v.set_schema_version("v9", cx));
    assert_eq!(
        d.read(&view, |v| v.schema_version().map(str::to_owned)),
        Some("v1beta1".into()),
        "an unknown version is ignored"
    );
    d.click("detail-schema-version-v1");
    assert!(
        row_keys(&mut d, &view).contains(&"spec.size".to_owned()),
        "spec stayed open"
    );
}

#[gpui::test]
fn the_button_opens_the_custom_resources_through_the_bus(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [widget_crd()]);
    let view = d.open(&crd_ref("widgets.example.com"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    d.f.dispatcher.clear();
    d.click("detail-open-resources");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::CrdOpenResources {
            cluster: cluster(),
            name: "widgets.example.com".into()
        }]
    );
}

#[gpui::test]
fn a_crd_without_a_schema_says_so(cx: &mut TestAppContext) {
    let fleet = Resource::from_json(fleet_crd_json()).unwrap();
    let mut d = Detail::new(cx, [fleet]);
    let view = d.open(&crd_ref("fleets.example.com"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    assert!(d.read(&view, |v| v.schema_versions().is_empty()));
    assert!(d.shown("detail-schema-none"));
    assert!(!d.shown("detail-schema-tree"));
}

#[gpui::test]
fn the_schema_follows_the_crd_when_it_changes_and_keeps_what_is_open(cx: &mut TestAppContext) {
    // The feed lists the CRD, and five seconds later the operator ships a new schema: a field is
    // added to `spec`.
    let mut json = crate::crds::tests::fixture::widget_crd_json();
    json["metadata"]["resourceVersion"] = json!("6");
    json["spec"]["versions"][2]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"]
        ["colour"] = json!({"type": "string", "description": "A colour."});
    let newer = Resource::from_json(json).unwrap();
    let mut d = Detail::new(cx, [widget_crd()]);
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            widget_crd(),
        ])])])
        .ok_at(
            Duration::from_secs(5),
            DeltaBatch::from_deltas(vec![Delta::Applied(newer)]),
        )
        .keep_open(),
    );
    let view = d.open(&crd_ref("widgets.example.com"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    d.click("schema-row-spec");
    let keys = row_keys(&mut d, &view);
    assert!(keys.contains(&"spec.size".to_owned()));
    assert!(!keys.contains(&"spec.colour".to_owned()));

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(5));
    d.settle();
    let keys = row_keys(&mut d, &view);
    assert!(keys.contains(&"spec.colour".to_owned()), "{keys:?}");
    assert!(keys.contains(&"spec.size".to_owned()), "spec stayed open");
}

#[gpui::test]
fn a_huge_schema_is_cut_and_only_the_rows_on_screen_are_built(cx: &mut TestAppContext) {
    let mut props = serde_json::Map::new();
    for i in 0..30_000 {
        props.insert(
            format!("field{i:05}"),
            json!({"type": "string", "description": "A field with a description."}),
        );
    }
    let mut json = crate::crds::tests::fixture::widget_crd_json();
    json["spec"]["versions"][2]["schema"]["openAPIV3Schema"] =
        json!({"type": "object", "properties": props});
    let mut d = Detail::new(cx, [Resource::from_json(json).unwrap()]);
    let view = d.open(&crd_ref("widgets.example.com"));
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    assert!(d.read(&view, |v| v.schema_truncated()));
    let rows = d.read(&view, |v| v.schema_rows().len());
    assert_eq!(rows, crate::crds::MAX_ROWS + 1);
    assert_eq!(
        d.read(&view, |v| v.schema_rows().last().map(|r| r.kind)),
        Some(RowKind::Truncated)
    );
    assert!(d.shown("schema-row-field00000"));
    assert!(!d.shown("schema-row-field01500"), "far rows are not built");
}

#[gpui::test]
fn the_crd_is_decoded_once_per_version_while_the_tab_is_up_and_not_at_all_otherwise(
    cx: &mut TestAppContext,
) {
    let mut json = crate::crds::tests::fixture::widget_crd_json();
    json["metadata"]["resourceVersion"] = json!("6");
    let newer = Resource::from_json(json).unwrap();
    let mut d = Detail::new(cx, [widget_crd()]);
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            widget_crd(),
        ])])])
        .ok_at(
            Duration::from_secs(5),
            DeltaBatch::from_deltas(vec![Delta::Applied(newer)]),
        )
        .keep_open(),
    );
    let view = d.open(&crd_ref("widgets.example.com"));
    // On the Overview nothing is decoded or kept, and the update changes nothing.
    assert_eq!(
        d.read(&view, |v| (v.schema.decodes, v.schema.is_decoded())),
        (0, false)
    );
    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(5));
    d.settle();
    assert_eq!(
        d.read(&view, |v| (v.schema.decodes, v.schema.is_decoded())),
        (0, false)
    );

    // Opening the tab reads the CRD once; clicks and version switches reuse the decoded tree.
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    assert_eq!(
        d.read(&view, |v| (v.schema.decodes, v.schema.is_decoded())),
        (1, true)
    );
    d.click("schema-row-spec");
    d.click("schema-row-spec");
    d.click("detail-schema-version-v1beta1");
    d.click("detail-schema-version-v1");
    assert!(row_keys(&mut d, &view).contains(&"spec".to_owned()));
    assert_eq!(d.read(&view, |v| v.schema.decodes), 1);

    // Leaving the tab frees the tree. Coming back keeps the rows (nothing changed), and the CRD
    // is decoded again only when a click needs the tree.
    d.update(&view, |v, cx| v.set_tab(DetailTab::Overview, cx));
    assert!(!d.read(&view, |v| v.schema.is_decoded()));
    d.update(&view, |v, cx| v.set_tab(DetailTab::Schema, cx));
    assert_eq!(d.read(&view, |v| v.schema.decodes), 1);
    d.click("schema-row-spec");
    assert_eq!(d.read(&view, |v| v.schema.decodes), 2);
}
