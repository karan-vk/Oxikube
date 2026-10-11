//! E10-S05: hover docs from the schema (`oxikube_editor::assist::hover_at`).

#[path = "assist/common.rs"]
mod common;

use common::*;
use oxikube_editor::yaml::JsonPath;

fn path(text: &str) -> JsonPath {
    text.parse().unwrap()
}

#[test]
fn a_field_shows_its_type_and_description() {
    let doc =
        hover("apiVersion: apps/v1\nkind: Deployment\nspec:\n  repli‸cas: 3\n").expect("hover");
    assert_eq!(doc.name, "replicas");
    assert_eq!(doc.path, path("spec.replicas"));
    assert_eq!(doc.type_label, "int or string");
    assert_eq!(
        doc.description.as_deref(),
        Some(
            "Number of desired pods. This is a pointer to distinguish between explicit zero and not specified. Defaults to 1."
        )
    );
    assert!(!doc.required);
}

#[test]
fn the_value_of_a_field_shows_the_same_docs_as_its_key() {
    let on_key = hover("kind: Pod\napiVersion: v1\nspec:\n  restartPo‸licy: Never\n").unwrap();
    let on_value = hover("kind: Pod\napiVersion: v1\nspec:\n  restartPolicy: Ne‸ver\n").unwrap();
    assert_eq!(on_key.path, on_value.path);
    assert_eq!(on_key.type_label, on_value.type_label);
    assert_ne!(
        on_key.range, on_value.range,
        "each underlines its own token"
    );
}

#[test]
fn an_enum_field_lists_its_values_and_default() {
    let doc = hover(
        "apiVersion: v1\nkind: Pod\nspec:\n  containers:\n    - name: a\n      imagePullPol‸icy: Always\n",
    )
    .unwrap();
    assert_eq!(doc.type_label, "string (enum)");
    assert_eq!(doc.enum_values, ["Always", "IfNotPresent", "Never"]);
    let doc = hover("apiVersion: v1\nkind: Pod\nspec:\n  restartPol‸icy: Always\n").unwrap();
    assert_eq!(doc.default.as_deref(), Some("Always"));
    let markdown = doc.to_markdown();
    assert!(
        markdown.contains("Allowed values: `Always`, `OnFailure`, `Never`"),
        "{markdown}"
    );
    assert!(markdown.contains("Default: `Always`"), "{markdown}");
}

#[test]
fn required_fields_are_marked_and_the_markdown_has_a_header() {
    let doc = hover("apiVersion: v1\nkind: Pod\nspec:\n  containers:\n    - na‸me: a\n").unwrap();
    assert!(doc.required);
    let markdown = doc.to_markdown();
    assert!(
        markdown.starts_with("**name** · `string` · required"),
        "{markdown}"
    );
    assert!(markdown.contains("Name of the container specified as a DNS_LABEL."));
    assert!(
        markdown.ends_with("`spec.containers[0].name`"),
        "{markdown}"
    );
    let port = hover(
        "apiVersion: v1\nkind: Pod\nspec:\n  containers:\n    - name: a\n      ports:\n        - containerPort: 8‸0\n",
    )
    .unwrap();
    assert!(port.to_markdown().contains("· `int32` · required"));
}

#[test]
fn it_works_inside_arrays() {
    let marked = POD.replace("image: busybox", "ima‸ge: busybox");
    let doc = hover(&marked).unwrap();
    assert_eq!(doc.path, path("spec.containers[1].image"));
    assert_eq!(doc.description.as_deref(), Some("Container image name."));
    let marked = POD.replace(
        "containerPort: 8080\n          protocol: TCP",
        "containerPort: 8080\n          proto‸col: TCP",
    );
    let doc = hover(&marked).unwrap();
    assert_eq!(doc.path, path("spec.containers[0].ports[0].protocol"));
    assert_eq!(doc.default.as_deref(), Some("TCP"));
}

#[test]
fn a_scalar_list_item_shows_the_item_type() {
    let doc = hover(
        "apiVersion: v1\nkind: Pod\nspec:\n  containers:\n    - name: a\n      command:\n        - ec‸ho\n",
    )
    .unwrap();
    assert_eq!(doc.name, "[0]");
    assert_eq!(doc.type_label, "string");
    assert_eq!(doc.path, path("spec.containers[0].command[0]"));
}

#[test]
fn a_map_value_shows_the_value_type() {
    let doc = hover("apiVersion: v1\nkind: Pod\nmetadata:\n  labels:\n    app: we‸b\n").unwrap();
    assert_eq!(doc.name, "app");
    assert_eq!(doc.type_label, "string");
    let labels = hover("apiVersion: v1\nkind: Pod\nmetadata:\n  lab‸els:\n    app: web\n").unwrap();
    assert_eq!(labels.type_label, "map of string");
}

#[test]
fn a_custom_resource_field_is_documented_by_its_crd() {
    let spec = hover("apiVersion: example.com/v1\nkind: Widget\nsp‸ec:\n  replicas: 2\n").unwrap();
    assert_eq!(spec.type_label, "object");
    assert_eq!(
        spec.description.as_deref(),
        Some("WidgetSpec defines the desired state of Widget.")
    );
    let replicas =
        hover("apiVersion: example.com/v1\nkind: Widget\nspec:\n  replic‸as: 2\n").unwrap();
    assert_eq!(replicas.type_label, "integer");
    // The CRD's recursive `template` is cut off by the flattening: the schema is not known there.
    assert_eq!(
        hover("apiVersion: example.com/v1\nkind: Widget\nspec:\n  template:\n    replic‸as: 2\n"),
        None
    );
    let config =
        hover("apiVersion: example.com/v1\nkind: Widget\nspec:\n  con‸fig:\n    a: 1\n").unwrap();
    assert_eq!(
        config.description.as_deref(),
        Some("Free-form config preserved verbatim by the API server.")
    );
}

#[test]
fn nothing_where_the_schema_says_nothing() {
    // Inside an object that preserves unknown fields.
    assert_eq!(
        hover("apiVersion: example.com/v1\nkind: Widget\nspec:\n  config:\n    fo‸o: 1\n"),
        None
    );
    // An unknown field (the validator's job).
    assert_eq!(
        hover("apiVersion: v1\nkind: Pod\nspec:\n  contianers‸: []\n"),
        None
    );
    // A kind without a schema, and a document without a kind.
    assert_eq!(
        hover("apiVersion: v1\nkind: Gadget\nspec:\n  rep‸licas: 1\n"),
        None
    );
    assert_eq!(hover("spec:\n  rep‸licas: 1\n"), None);
    // Whitespace, collections, the dash of a list item.
    assert_eq!(
        hover("apiVersion: v1\nkind: Pod\nspec:\n  ‸  restartPolicy: Never\n"),
        None
    );
    assert_eq!(
        hover("apiVersion: v1\nkind: Pod\nspec:\n  containers:\n  ‸- name: a\n"),
        None
    );
    assert_eq!(hover("apiVersion: v1\nkind: Pod\n‸\n"), None);
}

#[test]
fn it_picks_the_document_under_the_pointer() {
    let text = "apiVersion: v1\nkind: Pod\nspec:\n  restartPolicy: Never\n---\napiVersion: example.com/v1\nkind: Widget\nspec:\n  replic‸as: 1\n";
    let doc = hover(text).unwrap();
    assert_eq!(doc.type_label, "integer");
}

#[test]
fn broken_yaml_around_the_pointer_still_hovers() {
    let marked = "apiVersion: v1\nkind: Pod\nspec:\n  restartPo‸licy: Never\n  containers: [\n";
    assert!(hover(marked).is_some());
}
