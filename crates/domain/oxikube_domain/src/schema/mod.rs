//! JSON Schemas for Kubernetes objects (E10-S01).
//!
//! The manifest editor (E10) validates YAML against the cluster's own OpenAPI v3
//! schemas, not a bundled copy. This module holds the flattened, per-GVK view the
//! validator (E10-S03), hover and completion (E10-S05) read: [`JsonSchema`].
//!
//! Everything here is pure: parsing from [`serde_json::Value`], `$ref`/`allOf`
//! flattening and the root lookup for a [`Gvk`](crate::ids::Gvk) need no I/O, so the validator is
//! testable without a cluster. Fetching, caching and invalidation live in the
//! adapter (`oxikube_kube::openapi`) behind the `SchemaPort` trait in
//! `oxikube_ports`.
//!
//! # `$ref` and `allOf`
//!
//! Kubernetes wraps a single `$ref` in `allOf` to attach a description, and CRD
//! schemas can be recursive. [`flatten_schema`] resolves local
//! `#/components/schemas/<name>` references and merges `allOf` entries; a
//! reference cycle (or a nesting deeper than [`MAX_REF_DEPTH`] references or [`MAX_NESTING_DEPTH`] levels) stops with an
//! open node whose [`truncated`](JsonSchema::truncated) flag is set, so the
//! validator treats the subtree as unknown instead of looping.

use serde::{Deserialize, Serialize};

mod flatten;
mod properties;
mod root;

pub use flatten::flatten_schema;
pub use properties::Properties;
pub use root::root_schema_for;

/// How many `$ref`s may nest on one path before flattening stops with an open
/// node. The deepest real chains (a Pod inside a Deployment inside ...) are in
/// the teens; anything deeper is a cycle or a pathological document.
pub const MAX_REF_DEPTH: usize = 64;

/// How deep schema structure (properties, items, `allOf` entries) may nest
/// before flattening stops with an open node, bounding recursion on a hostile
/// document. References do not count; real schemas nest under a hundred levels.
pub const MAX_NESTING_DEPTH: usize = 256;

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
    pub properties: Properties,
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
        flatten_schema(value, &serde_json::Map::new())
    }
}

#[cfg(test)]
mod tests;
