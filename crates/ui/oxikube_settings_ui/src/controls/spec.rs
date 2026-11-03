//! [`ControlSpec`]: which control edits a field, derived from its schema node.

use gpui::SharedString;
use oxikube_settings::{COLOR_FORMAT, DYNAMIC_ENUM_KEY, FONTS};
use serde_json::Value;

use crate::page::FieldKind;
use crate::page::schema_walk::Definitions;

/// The hex branch of `ClusterColour`'s schema: how a colour field without `format: color` is
/// recognised.
const COLOR_PATTERN_PREFIX: &str = "^#(";

/// The bounds of a number field, from the schema's `minimum` and `maximum`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NumberBounds {
    /// Whether only whole numbers are accepted.
    pub integer: bool,
    /// The smallest accepted value.
    pub min: Option<f64>,
    /// The largest accepted value.
    pub max: Option<f64>,
}

/// The control a setting gets on the settings page, chosen by its schema type.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlSpec {
    /// A boolean: a switch.
    Switch,
    /// A fixed set of strings (`enum`, or `oneOf` of `const`s): one button per value.
    Choice(Vec<SharedString>),
    /// An integer or a number: a text input checked against the bounds.
    Number(NumberBounds),
    /// A string: a text input.
    Text,
    /// A colour string: an input with a swatch.
    Color,
    /// A font family: an input with a filtered list of the installed families.
    Font,
    /// An array of strings: a list editor with add, remove and reorder.
    List,
    /// A shape no control covers (nested objects, maps, unions): a read-only summary and "Edit
    /// in JSON".
    Json,
}

impl ControlSpec {
    /// The control for the schema node `node` (a property of the settings schema).
    pub(crate) fn of(node: &Value, definitions: &Definitions<'_>) -> Self {
        let resolved = definitions.resolve(node);
        if is_font(node) || is_font(resolved) {
            return Self::Font;
        }
        if is_color(node, definitions) || is_color(resolved, definitions) {
            return Self::Color;
        }
        match definitions.kind_of(node) {
            FieldKind::Toggle => Self::Switch,
            FieldKind::Choice(values) => Self::Choice(values),
            FieldKind::Integer => Self::Number(bounds(resolved, true)),
            FieldKind::Number => Self::Number(bounds(resolved, false)),
            FieldKind::Text => Self::Text,
            FieldKind::List if string_items(resolved, definitions) => Self::List,
            FieldKind::List | FieldKind::Map | FieldKind::Other => Self::Json,
        }
    }

    /// Whether the value is edited in `settings.json` rather than on the page.
    pub fn is_json_only(&self) -> bool {
        matches!(self, Self::Json)
    }

    /// Whether edits are written at once (switches and choices) rather than after a pause in
    /// typing.
    pub fn writes_immediately(&self) -> bool {
        matches!(self, Self::Switch | Self::Choice(_))
    }
}

fn is_font(node: &Value) -> bool {
    node.get(DYNAMIC_ENUM_KEY).and_then(Value::as_str) == Some(FONTS)
}

/// Whether `node` is marked as a colour: `format: "color"`, or a hex `pattern` on it or on one of
/// its `anyOf` branches.
fn is_color(node: &Value, definitions: &Definitions<'_>) -> bool {
    let marked = |node: &Value| {
        node.get("format").and_then(Value::as_str) == Some(COLOR_FORMAT)
            || node
                .get("pattern")
                .and_then(Value::as_str)
                .is_some_and(|pattern| pattern.starts_with(COLOR_PATTERN_PREFIX))
    };
    let branches = |node: &Value| {
        node.get("anyOf")
            .and_then(Value::as_array)
            .is_some_and(|branches| {
                branches
                    .iter()
                    .any(|branch| marked(definitions.resolve(branch)))
            })
    };
    marked(node) || branches(node)
}

fn bounds(node: &Value, integer: bool) -> NumberBounds {
    NumberBounds {
        integer,
        min: node.get("minimum").and_then(Value::as_f64),
        max: node.get("maximum").and_then(Value::as_f64),
    }
}

/// Whether `node` is an array whose items are strings.
fn string_items(node: &Value, definitions: &Definitions<'_>) -> bool {
    node.get("items")
        .map(|items| definitions.resolve(items))
        .and_then(|items| items.get("type"))
        .is_some_and(|ty| ty == "string")
}
