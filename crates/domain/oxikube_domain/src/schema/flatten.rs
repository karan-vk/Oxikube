//! `$ref` and `allOf` flattening: the pure parse of an OpenAPI v3 schema node
//! into a [`JsonSchema`] (E10-S01).

use super::{AdditionalProperties, JsonSchema, MAX_NESTING_DEPTH, MAX_REF_DEPTH, SchemaType, XK8s};

/// Flattens one schema `node` with `$ref`/`allOf` resolved against `components`
/// (the `components.schemas` map of the group document).
///
/// Only local references (`#/components/schemas/<name>`, with `~0`/`~1`
/// escapes) resolve; anything else becomes an open [`truncated`](JsonSchema::truncated)
/// node. A reference already on the resolution `stack`, more than
/// [`MAX_REF_DEPTH`] references nested on one path, or structure nested deeper
/// than [`MAX_NESTING_DEPTH`], also stops with an open truncated node, so
/// recursive CRD schemas always terminate.
pub fn flatten_schema(
    node: &serde_json::Value,
    components: &serde_json::Map<String, serde_json::Value>,
) -> JsonSchema {
    flatten_node(node, components, &mut Vec::new(), 0)
}

/// Merges `allOf` entries into one node: Kubernetes wraps a single `$ref` in
/// `allOf` to attach a description, so the common case is one resolved
/// reference plus the wrapper's own keywords (the last entry).
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
        merged.properties.merge(entry.properties);
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
    if depth > MAX_NESTING_DEPTH || stack.len() > MAX_REF_DEPTH {
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
        // A reference adds no structural depth: the target is the same position.
        stack.push(name);
        let resolved = flatten_node(target, components, stack, depth);
        stack.pop();
        return resolved;
    }
    if let Some(entries) = object.get("allOf").and_then(serde_json::Value::as_array) {
        let flattened: Vec<JsonSchema> = entries
            .iter()
            .map(|entry| flatten_node(entry, components, stack, depth + 1))
            .collect();
        // The node's own keywords (its description, `required`, `properties`, ...) merge
        // last, so siblings of `allOf` win over the entries.
        let mut entries = flattened;
        entries.push(parse_node(object, components, stack, depth));
        let mut merged = merge_all_of(entries);
        apply_nullable(&mut merged, object);
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
        schema.properties = properties
            .iter()
            .map(|(name, property)| {
                (
                    name.clone(),
                    flatten_node(property, components, stack, depth + 1),
                )
            })
            .collect();
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
    apply_unions(&mut schema, object, components, stack, depth);
    apply_nullable(&mut schema, object);
    if schema.xk8s.int_or_string && schema.types.is_empty() {
        // `x-kubernetes-int-or-string` with an `anyOf` of integer/string and no
        // `type` of its own: the value is an integer or a string.
        schema.types = vec![SchemaType::Integer, SchemaType::String];
    }
    schema
}

/// `anyOf`/`oneOf`: when the node names no type of its own and every alternative
/// constrains the type, the node's types are the union of the alternatives'
/// (`anyOf: [{type: integer}, {type: string}]`). Any other union stays
/// unconstrained: the validator must not reject what one alternative allows.
fn apply_unions(
    schema: &mut JsonSchema,
    object: &serde_json::Map<String, serde_json::Value>,
    components: &serde_json::Map<String, serde_json::Value>,
    stack: &mut Vec<String>,
    depth: usize,
) {
    let alternatives: Vec<JsonSchema> = ["anyOf", "oneOf"]
        .into_iter()
        .filter_map(|keyword| object.get(keyword)?.as_array())
        .flatten()
        .map(|entry| flatten_node(entry, components, stack, depth + 1))
        .collect();
    if alternatives.is_empty() || !schema.types.is_empty() {
        return;
    }
    if alternatives.iter().all(|alt| !alt.types.is_empty()) {
        for alt in &alternatives {
            for ty in &alt.types {
                if !schema.types.contains(ty) {
                    schema.types.push(*ty);
                }
            }
        }
    }
}

/// `nullable: true` additionally allows `null` for a typed node.
fn apply_nullable(schema: &mut JsonSchema, object: &serde_json::Map<String, serde_json::Value>) {
    let nullable = object
        .get("nullable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if nullable && !schema.types.is_empty() && !schema.types.contains(&SchemaType::Null) {
        schema.types.push(SchemaType::Null);
    }
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
