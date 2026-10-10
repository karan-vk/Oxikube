//! JSON Schemas for Kubernetes objects (E10-S01).
//!
//! The manifest editor (E10) validates YAML against the cluster's own OpenAPI v3
//! schemas, not a bundled copy. This module holds the flattened, per-GVK view the
//! validator (E10-S03), hover and completion (E10-S05) read: [`JsonSchema`].
//!
//! Everything here is pure: parsing from [`serde_json::Value`], `$ref`/`allOf`
//! flattening and the root lookup for a [`Gvk`] need no I/O, so the validator is
//! testable without a cluster. Fetching, caching and invalidation live in the
//! adapter (`oxikube_kube::openapi`) behind the `SchemaPort` trait in
//! `oxikube_ports`.
//!
//! # `$ref` and `allOf`
//!
//! Kubernetes wraps a single `$ref` in `allOf` to attach a description, and CRD
//! schemas can be recursive. [`flatten_schema`] resolves local
//! `#/components/schemas/<name>` references and merges `allOf` entries; a
//! reference cycle (or a nesting deeper than [`MAX_REF_DEPTH`]) stops with an
//! open node whose [`truncated`](JsonSchema::truncated) flag is set, so the
//! validator treats the subtree as unknown instead of looping.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::Gvk;

/// How deep `$ref`/`allOf` flattening may nest before it stops with an open node.
/// Real schemas nest a handful of levels; anything deeper is a cycle or a
/// pathological document.
pub const MAX_REF_DEPTH: usize = 32;

/// One JSON type of a [`JsonSchema`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaType {
    /// `null`.
    Null,
    /// `boolean`.
    Boolean,
    /// `object`.
    Object,
    /// `array`.
    Array,
    /// `number` (any numeric value).
    Number,
    /// `string`.
    String,
    /// `integer`.
    Integer,
}

impl SchemaType {
    /// The OpenAPI spelling (`"object"`, ...).
    pub const fn as_str(self) -> &'static str {
        match self {
            SchemaType::Null => "null",
            SchemaType::Boolean => "boolean",
            SchemaType::Object => "object",
            SchemaType::Array => "array",
            SchemaType::Number => "number",
            SchemaType::String => "string",
            SchemaType::Integer => "integer",
        }
    }

    /// Parses the OpenAPI spelling; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "null" => SchemaType::Null,
            "boolean" => SchemaType::Boolean,
            "object" => SchemaType::Object,
            "array" => SchemaType::Array,
            "number" => SchemaType::Number,
            "string" => SchemaType::String,
            "integer" => SchemaType::Integer,
            _ => return None,
        })
    }
}

impl std::fmt::Display for SchemaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What `additionalProperties` of a [`JsonSchema`] allows.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum AdditionalProperties {
    /// Extra properties are allowed (the JSON Schema default, and what
    /// `x-kubernetes-preserve-unknown-fields: true` means for CRDs).
    #[default]
    Allowed,
    /// Extra properties are rejected (`additionalProperties: false`).
    Forbidden,
    /// Extra properties must match this schema.
    Schema(Box<JsonSchema>),
}

/// The `x-kubernetes-*` extensions of one schema node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XK8s {
    /// `x-kubernetes-preserve-unknown-fields: true`: the object keeps fields the
    /// schema does not list, so unknown fields are not errors.
    pub preserve_unknown_fields: bool,
    /// `x-kubernetes-int-or-string: true`: the value is an integer or a string
    /// (usually paired with `anyOf: [{type: integer}, {type: string}]`).
    pub int_or_string: bool,
    /// `x-kubernetes-list-type` (`atomic`, `set` or `map`), if present.
    pub list_type: Option<String>,
    /// `x-kubernetes-list-map-keys`, for `list-type: map`.
    pub list_map_keys: Vec<String>,
    /// `x-kubernetes-embedded-resource: true`: the node holds an embedded
    /// complete object (a CRD `spec`/`status` stump).
    pub embedded_resource: bool,
}

/// One flattened JSON Schema node: the validator's view of a value.
///
/// Built with [`flatten_schema`] (references resolved) or [`from_value`](Self::from_value)
/// (one document node, references left open). Every collection is owned and
/// `Clone`, so the adapter shares finished schemas as `Arc<JsonSchema>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonSchema {
    /// Allowed JSON types, in first-seen order. Empty means unconstrained.
    pub types: Vec<SchemaType>,
    /// Named child schemas of an object, by property name.
    pub properties: BTreeMap<String, JsonSchema>,
    /// Element schema of an array, if the schema constrains it.
    pub items: Option<Box<JsonSchema>>,
    /// Allowed values (`enum`), if the schema constrains them.
    pub enum_values: Vec<serde_json::Value>,
    /// Required object properties, sorted and de-duplicated.
    pub required: Vec<String>,
    /// `pattern` for strings, if the schema constrains it.
    pub pattern: Option<String>,
    /// `format` for strings (`date-time`, `int32`, ...), if present.
    pub format: Option<String>,
    /// Human documentation of the field, if present.
    pub description: Option<String>,
    /// What object properties outside [`properties`](Self::properties) allow.
    pub additional_properties: AdditionalProperties,
    /// The `x-kubernetes-*` extensions of this node.
    pub xk8s: XK8s,
    /// `true` when `$ref` flattening stopped early (a reference cycle or
    /// [`MAX_REF_DEPTH`]): the subtree is incomplete and must be treated as
    /// unknown, never as an error source.
    pub truncated: bool,
}

impl JsonSchema {
    /// An unconstrained node: any value is valid.
    pub fn any() -> Self {
        Self::default()
    }

    /// Whether the node constrains nothing (no types, no properties, no enum,
    /// additional properties allowed, no extensions).
    pub fn is_any(&self) -> bool {
        self.types.is_empty()
            && self.properties.is_empty()
            && self.items.is_none()
            && self.enum_values.is_empty()
            && self.required.is_empty()
            && self.pattern.is_none()
            && self.format.is_none()
            && matches!(self.additional_properties, AdditionalProperties::Allowed)
            && self.xk8s == XK8s::default()
            && !self.truncated
    }

    /// Whether `name` is a known property of this node.
    pub fn has_property(&self, name: &str) -> bool {
        self.properties.contains_key(name)
    }

    /// Parses one schema document node without resolving `$ref`: nested inline
    /// schemas are parsed recursively, while a `$ref` (or an `allOf` entry
    /// holding one) becomes an open [`truncated`](Self::truncated) node. Use
    /// [`flatten_schema`] with the document's `components.schemas` when
    /// references must resolve.
    pub fn from_value(value: &serde_json::Value) -> Self {
        flatten_node(value, &empty_components(), &mut Vec::new(), 0)
    }
}

/// Parses `value` as a map of component schemas (the `components.schemas` of an
/// OpenAPI v3 group document); non-object entries are skipped.
fn empty_components() -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::new()
}

/// Flattens one schema `node` with `$ref`/`allOf` resolved against `components`
/// (the `components.schemas` map of the group document).
///
/// Only local references (`#/components/schemas/<name>`, with `~0`/`~1`
/// escapes) resolve; anything else becomes an open [`truncated`](JsonSchema::truncated)
/// node. A reference already on the resolution `stack`, or nesting deeper than
/// [`MAX_REF_DEPTH`], also stops with an open truncated node, so recursive CRD
/// schemas always terminate.
pub fn flatten_schema(
    node: &serde_json::Value,
    components: &serde_json::Map<String, serde_json::Value>,
) -> JsonSchema {
    flatten_node(node, components, &mut Vec::new(), 0)
}

/// Finds the root schema for `gvk` in an OpenAPI v3 group `document` and
/// flattens it with [`flatten_schema`].
///
/// The root is the entry of `components.schemas` whose
/// `x-kubernetes-group-version-kind` list contains `{group, kind, version}`
/// equal to `gvk` (matched by value, never by name guessing). `None` when the
/// document has no such entry.
pub fn root_schema_for(document: &serde_json::Value, gvk: &Gvk) -> Option<JsonSchema> {
    let schemas = document.get("components")?.get("schemas")?.as_object()?;
    let root = schemas.values().find(|schema| {
        schema
            .get("x-kubernetes-group-version-kind")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|gvks| {
                gvks.iter().any(|entry| {
                    entry
                        .get("group")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        == &*gvk.group
                        && entry
                            .get("kind")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            == &*gvk.kind
                        && entry
                            .get("version")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            == &*gvk.version
                })
            })
    })?;
    Some(flatten_schema(root, schemas))
}

/// Merges `allOf` entries into one node: Kubernetes wraps a single `$ref` in
/// `allOf` to attach a description, so the common case is one resolved
/// reference plus sibling annotations.
///
/// Merge rules: types are the first non-empty list; properties merge with later
/// entries winning on name clashes; `required` unions; `items` is the last
/// present; `enum` is the first non-empty; `pattern`/`format`/`description`
/// are the last present; `additionalProperties` is the last non-default;
/// `XK8s` ORs the flags and takes the last `list_type` with unioned map keys;
/// `truncated` ORs.
fn merge_all_of(entries: Vec<JsonSchema>) -> JsonSchema {
    let mut merged = JsonSchema::default();
    for entry in entries {
        if merged.types.is_empty() && !entry.types.is_empty() {
            merged.types = entry.types;
        }
        merged.properties.extend(entry.properties);
        for name in entry.required {
            if !merged.required.contains(&name) {
                merged.required.push(name);
            }
        }
        if entry.items.is_some() {
            merged.items = entry.items;
        }
        if merged.enum_values.is_empty() && !entry.enum_values.is_empty() {
            merged.enum_values = entry.enum_values;
        }
        if entry.pattern.is_some() {
            merged.pattern = entry.pattern;
        }
        if entry.format.is_some() {
            merged.format = entry.format;
        }
        if entry.description.is_some() {
            merged.description = entry.description;
        }
        if !matches!(entry.additional_properties, AdditionalProperties::Allowed) {
            merged.additional_properties = entry.additional_properties;
        }
        if entry.xk8s.preserve_unknown_fields {
            merged.xk8s.preserve_unknown_fields = true;
        }
        if entry.xk8s.int_or_string {
            merged.xk8s.int_or_string = true;
            if merged.types.is_empty() {
                merged.types = vec![SchemaType::Integer, SchemaType::String];
            }
        }
        if entry.xk8s.list_type.is_some() {
            merged.xk8s.list_type = entry.xk8s.list_type;
        }
        for key in entry.xk8s.list_map_keys {
            if !merged.xk8s.list_map_keys.contains(&key) {
                merged.xk8s.list_map_keys.push(key);
            }
        }
        if entry.xk8s.embedded_resource {
            merged.xk8s.embedded_resource = true;
        }
        merged.truncated |= entry.truncated;
    }
    merged.required.sort();
    merged
}

fn flatten_node(
    node: &serde_json::Value,
    components: &serde_json::Map<String, serde_json::Value>,
    stack: &mut Vec<String>,
    depth: usize,
) -> JsonSchema {
    if depth > MAX_REF_DEPTH {
        return truncated_any();
    }
    let Some(object) = node.as_object() else {
        return JsonSchema::any();
    };
    if let Some(name) = ref_name(object) {
        if stack.contains(&name) {
            return truncated_any();
        }
        let Some(target) = components.get(&name) else {
            return truncated_any();
        };
        stack.push(name);
        let resolved = flatten_node(target, components, stack, depth + 1);
        stack.pop();
        return resolved;
    }
    if let Some(entries) = object.get("allOf").and_then(serde_json::Value::as_array) {
        let flattened: Vec<JsonSchema> = entries
            .iter()
            .map(|entry| flatten_node(entry, components, stack, depth + 1))
            .collect();
        let mut merged = merge_all_of(flattened);
        // Sibling annotations beside `allOf` (the description the wrapper carries)
        // win over the merged entries.
        if let Some(description) = object
            .get("description")
            .and_then(serde_json::Value::as_str)
        {
            merged.description = Some(description.to_owned());
        }
        let sibling_xk8s = parse_xk8s(object);
        if sibling_xk8s.preserve_unknown_fields {
            merged.xk8s.preserve_unknown_fields = true;
        }
        if sibling_xk8s.int_or_string && merged.types.is_empty() {
            merged.types = vec![SchemaType::Integer, SchemaType::String];
        }
        return merged;
    }
    parse_node(object, components, stack, depth)
}

fn truncated_any() -> JsonSchema {
    JsonSchema {
        truncated: true,
        ..JsonSchema::any()
    }
}

/// The component name of a local `#/components/schemas/<name>` reference, or
/// `None` when `$ref` is absent or not local.
fn ref_name(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let reference = object.get("$ref")?.as_str()?;
    let name = reference.strip_prefix("#/components/schemas/")?;
    Some(name.replace("~1", "/").replace("~0", "~"))
}

fn parse_node(
    object: &serde_json::Map<String, serde_json::Value>,
    components: &serde_json::Map<String, serde_json::Value>,
    stack: &mut Vec<String>,
    depth: usize,
) -> JsonSchema {
    let mut schema = JsonSchema::default();
    if let Some(types) = object.get("type") {
        let texts: Vec<&str> = match types {
            serde_json::Value::String(single) => vec![single.as_str()],
            serde_json::Value::Array(many) => {
                many.iter().filter_map(serde_json::Value::as_str).collect()
            }
            _ => Vec::new(),
        };
        for text in texts {
            if let Some(parsed) = SchemaType::parse(text) {
                if !schema.types.contains(&parsed) {
                    schema.types.push(parsed);
                }
            }
        }
    }
    if let Some(properties) = object
        .get("properties")
        .and_then(serde_json::Value::as_object)
    {
        for (name, property) in properties {
            schema.properties.insert(
                name.clone(),
                flatten_node(property, components, stack, depth + 1),
            );
        }
    }
    if let Some(items) = object.get("items") {
        schema.items = Some(Box::new(flatten_node(items, components, stack, depth + 1)));
    }
    if let Some(values) = object.get("enum").and_then(serde_json::Value::as_array) {
        schema.enum_values = values.clone();
    }
    if let Some(required) = object.get("required").and_then(serde_json::Value::as_array) {
        for name in required.iter().filter_map(serde_json::Value::as_str) {
            if !schema.required.contains(&name.to_owned()) {
                schema.required.push(name.to_owned());
            }
        }
        schema.required.sort();
    }
    if let Some(pattern) = object.get("pattern").and_then(serde_json::Value::as_str) {
        schema.pattern = Some(pattern.to_owned());
    }
    if let Some(format) = object.get("format").and_then(serde_json::Value::as_str) {
        schema.format = Some(format.to_owned());
    }
    if let Some(description) = object
        .get("description")
        .and_then(serde_json::Value::as_str)
    {
        schema.description = Some(description.to_owned());
    }
    schema.additional_properties = parse_additional(object, components, stack, depth);
    schema.xk8s = parse_xk8s(object);
    if schema.xk8s.int_or_string && schema.types.is_empty() {
        // `x-kubernetes-int-or-string` with an `anyOf` of integer/string and no
        // `type` of its own: the value is an integer or a string.
        schema.types = vec![SchemaType::Integer, SchemaType::String];
    }
    schema
}

fn parse_additional(
    object: &serde_json::Map<String, serde_json::Value>,
    components: &serde_json::Map<String, serde_json::Value>,
    stack: &mut Vec<String>,
    depth: usize,
) -> AdditionalProperties {
    match object.get("additionalProperties") {
        None => AdditionalProperties::Allowed,
        Some(serde_json::Value::Bool(true)) => AdditionalProperties::Allowed,
        Some(serde_json::Value::Bool(false)) => AdditionalProperties::Forbidden,
        Some(schema) => AdditionalProperties::Schema(Box::new(flatten_node(
            schema,
            components,
            stack,
            depth + 1,
        ))),
    }
}

fn parse_xk8s(object: &serde_json::Map<String, serde_json::Value>) -> XK8s {
    let flag = |name: &str| {
        object
            .get(name)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    XK8s {
        preserve_unknown_fields: flag("x-kubernetes-preserve-unknown-fields"),
        int_or_string: flag("x-kubernetes-int-or-string"),
        list_type: object
            .get("x-kubernetes-list-type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        list_map_keys: object
            .get("x-kubernetes-list-map-keys")
            .and_then(serde_json::Value::as_array)
            .map(|keys| {
                keys.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        embedded_resource: flag("x-kubernetes-embedded-resource"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn deployment_document() -> serde_json::Value {
        json!({
            "components": {
                "schemas": {
                    "io.k8s.api.apps.v1.Deployment": {
                        "description": "Deployment enables declarative updates for Pods.",
                        "properties": {
                            "apiVersion": {"type": "string"},
                            "kind": {"type": "string"},
                            "spec": {"$ref": "#/components/schemas/io.k8s.api.apps.v1.DeploymentSpec"}
                        },
                        "required": ["spec"],
                        "type": "object",
                        "x-kubernetes-group-version-kind": [
                            {"group": "apps", "kind": "Deployment", "version": "v1"}
                        ]
                    },
                    "io.k8s.api.apps.v1.DeploymentSpec": {
                        "properties": {
                            "replicas": {
                                "allOf": [{"$ref": "#/components/schemas/io.k8s.apimachinery.pkg.apis.meta.v1.LabelSelector"}],
                                "description": "stale wrapper kept to test allOf beside a sibling"
                            },
                            "strategy": {"type": "object"}
                        },
                        "type": "object"
                    },
                    "io.k8s.apimachinery.pkg.apis.meta.v1.LabelSelector": {
                        "properties": {
                            "matchLabels": {
                                "additionalProperties": {"type": "string"},
                                "type": "object"
                            }
                        },
                        "type": "object"
                    }
                }
            }
        })
    }

    #[test]
    fn root_lookup_matches_by_gvk_value_not_name() {
        let document = deployment_document();
        let gvk = Gvk::new("apps", "v1", "Deployment");
        let schema = root_schema_for(&document, &gvk).expect("deployment schema");
        assert!(schema.has_property("spec"));
        assert_eq!(schema.required, vec!["spec".to_owned()]);
        assert_eq!(
            schema.description.as_deref(),
            Some("Deployment enables declarative updates for Pods.")
        );
        assert!(root_schema_for(&document, &Gvk::new("apps", "v1", "StatefulSet")).is_none());
    }

    #[test]
    fn all_of_wrapper_resolves_the_reference() {
        let document = deployment_document();
        let schemas = document["components"]["schemas"].as_object().unwrap();
        let spec = flatten_schema(
            &document["components"]["schemas"]["io.k8s.api.apps.v1.DeploymentSpec"],
            schemas,
        );
        let replicas = spec.properties.get("replicas").expect("replicas");
        assert!(
            replicas.has_property("matchLabels"),
            "allOf $ref must resolve to LabelSelector"
        );
        assert_eq!(
            replicas.description.as_deref(),
            Some("stale wrapper kept to test allOf beside a sibling"),
            "the sibling description beside allOf wins"
        );
    }

    #[test]
    fn nested_refs_flatten_transitively() {
        let document = deployment_document();
        let schema =
            root_schema_for(&document, &Gvk::new("apps", "v1", "Deployment")).expect("schema");
        let spec = schema.properties.get("spec").expect("spec");
        let replicas = spec.properties.get("replicas").expect("replicas");
        let match_labels = replicas.properties.get("matchLabels").expect("matchLabels");
        assert!(matches!(
            match_labels.additional_properties,
            AdditionalProperties::Schema(_)
        ));
    }

    #[test]
    fn recursive_crd_schema_terminates_truncated() {
        let document = json!({
            "components": {
                "schemas": {
                    "example.com.v1.Widget": {
                        "properties": {
                            "child": {"$ref": "#/components/schemas/example.com.v1.Widget"}
                        },
                        "type": "object",
                        "x-kubernetes-group-version-kind": [
                            {"group": "example.com", "kind": "Widget", "version": "v1"}
                        ]
                    }
                }
            }
        });
        let schema = root_schema_for(&document, &Gvk::new("example.com", "v1", "Widget"))
            .expect("widget schema");
        // One unfolding resolves; the cycle stops at the next level with an
        // open node, so validation never loops.
        let child = schema.properties.get("child").expect("child");
        assert!(!child.truncated, "one unfolding resolves");
        let grandchild = child.properties.get("child").expect("grandchild");
        assert!(
            grandchild.truncated,
            "the recursive reference must stop open"
        );
        assert!(!schema.truncated, "the root itself resolved");
    }

    #[test]
    fn preserve_unknown_fields_marks_the_node_open() {
        let document = json!({
            "components": {
                "schemas": {
                    "example.com.v1.Raw": {
                        "type": "object",
                        "x-kubernetes-preserve-unknown-fields": true,
                        "x-kubernetes-group-version-kind": [
                            {"group": "example.com", "kind": "Raw", "version": "v1"}
                        ]
                    }
                }
            }
        });
        let schema =
            root_schema_for(&document, &Gvk::new("example.com", "v1", "Raw")).expect("schema");
        assert!(schema.xk8s.preserve_unknown_fields);
    }

    #[test]
    fn int_or_string_recognised_with_any_of() {
        let document = json!({
            "components": {
                "schemas": {
                    "io.k8s.IntOrString": {
                        "anyOf": [{"type": "integer"}, {"type": "string"}],
                        "x-kubernetes-int-or-string": true,
                        "x-kubernetes-group-version-kind": [
                            {"group": "", "kind": "IntOrString", "version": "v1"}
                        ]
                    }
                }
            }
        });
        let schema =
            root_schema_for(&document, &Gvk::new("", "v1", "IntOrString")).expect("schema");
        assert!(schema.xk8s.int_or_string);
        assert!(schema.types.contains(&SchemaType::Integer));
        assert!(schema.types.contains(&SchemaType::String));
    }

    #[test]
    fn missing_reference_stays_open_not_missing() {
        let schemas = empty_components();
        let schema = flatten_schema(
            &json!({"$ref": "#/components/schemas/io.k8s.DoesNotExist"}),
            &schemas,
        );
        assert!(schema.truncated);
    }

    #[test]
    fn schema_type_spellings_round_trip() {
        for (text, parsed) in [
            ("object", SchemaType::Object),
            ("array", SchemaType::Array),
            ("integer", SchemaType::Integer),
            ("string", SchemaType::String),
        ] {
            assert_eq!(SchemaType::parse(text), Some(parsed));
            assert_eq!(parsed.as_str(), text);
        }
        assert_eq!(SchemaType::parse("unknown"), None);
    }
}
