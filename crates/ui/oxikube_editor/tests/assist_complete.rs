//! E10-S05: completion from the schema (`oxikube_editor::assist::complete_at`).

#[path = "assist/common.rs"]
mod common;

use common::*;
use oxikube_editor::assist::CompletionKind;
use oxikube_editor::validate::{ValidateOptions, validate_buffer};
use oxikube_editor::yaml::parse;

const HEAD: &str = "apiVersion: v1\nkind: Pod\n";

#[test]
fn a_key_position_lists_the_siblings_not_already_present() {
    let (_, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - name: a\n      ‸\n"
    ));
    let labels = labels(&items);
    assert!(labels.contains(&"image"), "{labels:?}");
    assert!(labels.contains(&"imagePullPolicy"));
    assert!(
        !labels.contains(&"name"),
        "name is already there: {labels:?}"
    );
    assert!(items.iter().all(|i| i.kind != CompletionKind::Value));
}

#[test]
fn required_fields_come_first() {
    let (_, items) = complete(&format!("{HEAD}spec:\n  containers:\n    - ‸\n"));
    let fields: Vec<_> = items
        .iter()
        .filter(|i| i.kind == CompletionKind::Field)
        .collect();
    assert_eq!(fields[0].label, "name", "{:?}", labels(&items));
    assert!(fields[0].required);
    assert!(fields[0].detail.as_deref().unwrap().ends_with("required"));
    assert!(fields[1..].iter().all(|f| !f.required));
    let rest: Vec<_> = fields[1..].iter().map(|f| f.label.as_str()).collect();
    let mut sorted = rest.clone();
    sorted.sort_unstable();
    assert_eq!(rest, sorted, "the rest is alphabetical");
}

#[test]
fn the_root_offers_the_kinds_fields_but_not_status() {
    let (_, items) = complete(&format!("{HEAD}‸"));
    let labels = labels(&items);
    assert!(
        labels.contains(&"metadata") && labels.contains(&"spec"),
        "{labels:?}"
    );
    assert!(!labels.contains(&"status"), "the server owns status");
    assert!(!labels.contains(&"kind") && !labels.contains(&"apiVersion"));
}

#[test]
fn a_value_position_offers_the_enum_members() {
    let (text, items) = complete(&format!("{HEAD}spec:\n  restartPolicy: ‸\n"));
    assert_eq!(labels(&items), ["Always", "OnFailure", "Never"]);
    assert!(items.iter().all(|i| i.kind == CompletionKind::Value));
    assert_eq!(
        item(&items, "Always").detail.as_deref(),
        Some("string (enum) · default")
    );
    assert_eq!(
        apply(&text, item(&items, "Never")),
        format!("{HEAD}spec:\n  restartPolicy: Never\n")
    );
}

#[test]
fn a_typed_value_prefix_filters_and_the_rest_of_the_value_is_replaced() {
    let (text, items) = complete(&format!("{HEAD}spec:\n  restartPolicy: On‸Fail\n"));
    assert_eq!(labels(&items), ["OnFailure"]);
    assert_eq!(
        apply(&text, &items[0]),
        format!("{HEAD}spec:\n  restartPolicy: OnFailure\n")
    );
    let (_, none) = complete(&format!("{HEAD}spec:\n  restartPolicy: Zz‸\n"));
    assert!(none.is_empty());
}

#[test]
fn a_colon_without_a_space_gets_one() {
    let (text, items) = complete(&format!("{HEAD}spec:\n  restartPolicy:‸\n"));
    assert_eq!(
        apply(&text, item(&items, "Never")),
        format!("{HEAD}spec:\n  restartPolicy: Never\n")
    );
}

#[test]
fn booleans_offer_true_and_false() {
    let (text, items) = complete(&format!("{HEAD}spec:\n  hostNetwork: ‸\n"));
    assert_eq!(labels(&items), ["true", "false"]);
    assert_eq!(
        apply(&text, &items[1]),
        format!("{HEAD}spec:\n  hostNetwork: false\n")
    );
    let (_, items) = complete(&format!("{HEAD}spec:\n  hostNetwork: t‸\n"));
    assert_eq!(labels(&items), ["true"]);
}

#[test]
fn a_value_inside_a_list_item_resolves_through_the_item_schema() {
    let (_, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - name: a\n      imagePullPolicy: ‸\n"
    ));
    assert_eq!(labels(&items), ["Always", "IfNotPresent", "Never"]);
    // On the dash line itself.
    let (_, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - name: a\n    - imagePullPolicy: I‸\n"
    ));
    assert_eq!(labels(&items), ["IfNotPresent"]);
}

#[test]
fn inside_the_second_list_item_the_item_schema_is_used() {
    let (_, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - name: a\n      image: x\n    - name: b\n      ‸\n"
    ));
    let labels = labels(&items);
    assert!(
        labels.contains(&"image") && labels.contains(&"ports"),
        "{labels:?}"
    );
    assert!(
        !labels.contains(&"name"),
        "the second item already has a name"
    );
    // Keys present in the first item do not count for the second.
    let (_, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - name: a\n      image: x\n    - name: b\n      ima‸\n"
    ));
    assert!(common::labels(&items).contains(&"image"));
}

#[test]
fn a_half_typed_key_completes_from_the_partial_tree() {
    let (text, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - name: a\n      ima‸\n"
    ));
    assert_eq!(labels(&items), ["image", "imagePullPolicy"]);
    assert_eq!(
        apply(&text, item(&items, "image")),
        format!("{HEAD}spec:\n  containers:\n    - name: a\n      image: \n")
    );
    assert_eq!(items[0].replace, text.len() - 4..text.len() - 1);
}

#[test]
fn a_prefix_in_the_middle_of_a_buffer_and_of_a_word() {
    let (text, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - name: a\n      ima‸ge: nginx\n      stdin: true\n"
    ));
    assert_eq!(labels(&items), ["image", "imagePullPolicy"]);
    // A colon follows the word: the completion inserts the name alone and swallows the rest of it.
    assert_eq!(
        apply(&text, item(&items, "imagePullPolicy")),
        format!(
            "{HEAD}spec:\n  containers:\n    - name: a\n      imagePullPolicy: nginx\n      stdin: true\n"
        )
    );
}

#[test]
fn objects_and_lists_break_the_line_after_the_key() {
    let (text, items) = complete(&format!("{HEAD}spec:\n  cont‸\n"));
    assert_eq!(
        apply(&text, item(&items, "containers")),
        format!("{HEAD}spec:\n  containers:\n    - \n")
    );
    let (text, items) = complete(&format!("{HEAD}metadata:\n  lab‸\n"));
    assert_eq!(
        apply(&text, item(&items, "labels")),
        format!("{HEAD}metadata:\n  labels:\n    \n")
    );
}

#[test]
fn a_blank_line_after_a_key_without_a_value_opens_its_object() {
    let (_, items) = complete(&format!("{HEAD}spec:\n  ‸\n"));
    let labels = labels(&items);
    assert!(
        labels.contains(&"containers") && labels.contains(&"restartPolicy"),
        "{labels:?}"
    );
}

#[test]
fn a_map_offers_no_keys() {
    let (_, items) = complete(&format!("{HEAD}metadata:\n  labels:\n    ‸\n"));
    assert!(items.is_empty(), "{:?}", labels(&items));
    let (_, items) = complete(&format!("{HEAD}metadata:\n  labels:\n    app: ‸\n"));
    assert!(items.is_empty());
}

#[test]
fn nothing_where_the_schema_is_open() {
    let (_, items) =
        complete("apiVersion: example.com/v1\nkind: Widget\nspec:\n  config:\n    ‸\n");
    assert!(items.is_empty());
    let (_, items) =
        complete("apiVersion: example.com/v1\nkind: Widget\nspec:\n  config:\n    fo‸\n");
    assert!(items.is_empty());
    // The CRD's recursive template is cut off too.
    let (_, items) =
        complete("apiVersion: example.com/v1\nkind: Widget\nspec:\n  template:\n    ‸\n");
    assert!(items.is_empty());
}

#[test]
fn a_custom_resource_completes_from_its_crd() {
    let (_, items) = complete("apiVersion: example.com/v1\nkind: Widget\nspec:\n  ‸\n");
    assert_eq!(labels(&items), ["config", "replicas", "template"]);
    assert_eq!(item(&items, "replicas").detail.as_deref(), Some("integer"));
    let (_, items) = complete("apiVersion: example.com/v1\nkind: Widget\n‸\n");
    assert_eq!(labels(&items), ["metadata", "spec"]);
}

#[test]
fn nothing_in_comments_block_scalars_or_flow_collections() {
    for marked in [
        format!("{HEAD}spec:\n  # res‸\n"),
        format!("{HEAD}spec:\n  restartPolicy: Always # Al‸\n"),
        format!("{HEAD}spec:\n  containers:\n    - name: a\n      command: [\"sh\", ‸]\n"),
        format!("{HEAD}metadata:\n  annotations:\n    note: |\n      ima‸\n"),
        format!(
            "{HEAD}spec:\n  containers:\n    - name: a\n      command:\n        - |\n          ima‸\n"
        ),
    ] {
        let (_, items) = complete(&marked);
        assert!(items.is_empty(), "{marked}: {:?}", labels(&items));
    }
}

#[test]
fn a_kind_without_a_schema_offers_only_the_two_fields_every_object_has() {
    let (_, items) = complete("apiVersion: v1\nkind: Gadget\nspec:\n  ‸\n");
    assert!(items.is_empty());
    let (_, items) = complete("‸\n");
    assert_eq!(labels(&items), ["apiVersion", "kind"]);
    let (_, items) = complete("apiVersion: v1\nk‸\n");
    assert_eq!(labels(&items), ["kind"]);
}

#[test]
fn the_document_under_the_cursor_decides_in_a_multi_document_buffer() {
    let text = "apiVersion: v1\nkind: Pod\nspec:\n  restartPolicy: Never\n---\napiVersion: example.com/v1\nkind: Widget\nspec:\n  ‸\n";
    let (_, items) = complete(text);
    assert_eq!(labels(&items), ["config", "replicas", "template"]);
}

#[test]
fn a_list_item_start_offers_the_container_block_first() {
    let (text, items) = complete(&format!("{HEAD}spec:\n  containers:\n    - ‸\n"));
    assert_eq!(items[0].label, "container");
    assert_eq!(items[0].kind, CompletionKind::Snippet);
    let applied = apply(&text, &items[0]);
    assert!(
        applied.starts_with(&format!(
            "{HEAD}spec:\n  containers:\n    - name: app\n      image: nginx:latest\n      ports:\n        - containerPort: 80\n"
        )),
        "{applied}"
    );
    // Only on an empty item.
    let (_, items) = complete(&format!(
        "{HEAD}spec:\n  containers:\n    - ‸\n      name: a\n"
    ));
    assert!(items.iter().all(|i| i.kind != CompletionKind::Snippet));
}

#[test]
fn the_block_snippets_are_offered_for_the_lists_a_spec_can_hold() {
    let (text, items) = complete(&format!("{HEAD}spec:\n  vol‸\n"));
    let labels = labels(&items);
    assert_eq!(
        labels,
        [
            "volumes",
            "volume (emptyDir)",
            "volume (configMap)",
            "volume (persistentVolumeClaim)"
        ]
    );
    let applied = apply(&text, item(&items, "volume (configMap)"));
    assert_eq!(
        applied,
        format!(
            "{HEAD}spec:\n  volumes:\n    - name: config\n      configMap:\n        name: app-config\n"
        )
    );
    // A list that is already there is added to at its items, not by a second `volumes:`.
    let (_, items) = complete(&format!("{HEAD}spec:\n  volumes: []\n  vol‸\n"));
    assert!(items.is_empty() || items.iter().all(|i| i.label != "volumes"));
    let (_, items) = complete(&format!("{HEAD}spec:\n  cont‸\n"));
    // `cont` is a prefix of `containers` and `container`, and inside `initContainers`.
    assert_eq!(
        common::labels(&items),
        [
            "containers",
            "initContainers",
            "container",
            "init container"
        ]
    );
    let (_, items) = complete(&format!("{HEAD}spec:\n  volumes:\n    - ‸\n"));
    assert_eq!(items[0].label, "volume (emptyDir)");
}

#[test]
fn every_snippet_inserts_yaml_the_schema_accepts() {
    for start in [
        format!("{HEAD}spec:\n  cont‸\n"),
        format!("{HEAD}spec:\n  vol‸\n"),
        format!("{HEAD}spec:\n  init‸\n"),
        format!("{HEAD}spec:\n  containers:\n    - name: first\n      image: x\n    - ‸\n"),
        format!("{HEAD}spec:\n  containers:\n    - name: first\n  volumes:\n    - ‸\n"),
    ] {
        let (text, items) = complete(&start);
        let snippets: Vec<_> = items
            .iter()
            .filter(|i| i.kind == CompletionKind::Snippet)
            .collect();
        assert!(!snippets.is_empty(), "{start}");
        for snippet in snippets {
            let applied = apply(&text, snippet);
            let parsed = parse(&applied);
            let diags = validate_buffer(&parsed, schemas, &ValidateOptions::default());
            let wrong: Vec<_> = diags
                .iter()
                .filter(|d| !(d.code.as_str() == "required" && d.message.contains("containers")))
                .collect();
            assert!(wrong.is_empty(), "{}\n{applied}\n{wrong:#?}", snippet.label);
        }
    }
}

#[test]
fn enum_members_that_would_read_as_other_types_are_quoted() {
    use oxikube_domain::ids::Gvk;
    use oxikube_domain::schema::JsonSchema;
    use oxikube_editor::assist::{Buffer, complete_at};
    use std::sync::Arc;
    let schema = Arc::new(JsonSchema::from_value(&serde_json::json!({
        "type": "object",
        "properties": {
            "mode": {"type": "string", "enum": ["on", "1.5", "plain", ""]},
            "level": {"type": "integer", "enum": [1, 2]},
        }
    })));
    let for_all = |_: &Gvk| Some(schema.clone());
    let text = "apiVersion: v1\nkind: K\nmode: ";
    let items = complete_at(&Buffer::new(text), for_all, text.len());
    let inserted: Vec<_> = items.iter().map(|i| i.insert_text.as_str()).collect();
    assert_eq!(inserted, ["\"on\"", "\"1.5\"", "plain", "\"\""]);
    let text = "apiVersion: v1\nkind: K\nlevel: ";
    let items = complete_at(&Buffer::new(text), for_all, text.len());
    let inserted: Vec<_> = items.iter().map(|i| i.insert_text.as_str()).collect();
    assert_eq!(inserted, ["1", "2"], "integers stay bare");
}

#[test]
fn completion_never_panics_on_any_cursor_of_a_manifest() {
    for text in [
        POD,
        "a: 1\n\tb: 2\n",
        "- - a\n  - b\n",
        "k: 'é\n",
        "{a: [",
        "---\n---\n",
    ] {
        for at in 0..=text.len() {
            if text.is_char_boundary(at) {
                let _ = oxikube_editor::assist::complete_at(
                    &oxikube_editor::assist::Buffer::new(text),
                    schemas,
                    at,
                );
            }
        }
    }
}
