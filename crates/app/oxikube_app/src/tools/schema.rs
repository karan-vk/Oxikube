//! The JSON Schema subset tool arguments are checked against before a tool runs.
//!
//! Oxikube's tool schemas are flat objects, so the check covers what they use: `type` (`object`,
//! `string`, `integer`, `number`, `boolean`, `array`), `required`, `properties`,
//! `additionalProperties: false`, `enum`, `minimum` / `maximum`, `minLength` / `maxLength` and
//! array `items`. Anything else in a schema is ignored, not rejected. The error names the first
//! offending argument.

use oxikube_domain::{OxiError, OxiResult};
use serde_json::Value;

/// Checks `args` against `schema`.
///
/// # Errors
///
/// A validation error naming the argument that does not fit.
pub fn validate_args(schema: &Value, args: &Value) -> OxiResult<()> {
    check(schema, args, "arguments")
}

fn check(schema: &Value, value: &Value, at: &str) -> OxiResult<()> {
    let bad = |why: String| Err(OxiError::validation(format!("{at}: {why}")));
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let fits = match kind {
            "object" => value.is_object(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            _ => true,
        };
        if !fits {
            return bad(format!("expected {kind}"));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        let names: Vec<String> = allowed.iter().map(Value::to_string).collect();
        return bad(format!("expected one of {}", names.join(", ")));
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64)
            && n < min
        {
            return bad(format!("must be at least {min}"));
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64)
            && n > max
        {
            return bad(format!("must be at most {max}"));
        }
    }
    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64)
            && len < min
        {
            return bad(format!("must be at least {min} characters"));
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64)
            && len > max
        {
            return bad(format!("must be at most {max} characters"));
        }
    }
    if let Some(items) = value.as_array()
        && let Some(item_schema) = schema.get("items")
    {
        for (ix, item) in items.iter().enumerate() {
            check(item_schema, item, &format!("{at}[{ix}]"))?;
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(name) {
                    return bad(format!("missing required argument {name:?}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (name, member) in object {
            match properties.and_then(|p| p.get(name)) {
                Some(property) => check(property, member, &format!("{at}.{name}"))?,
                None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    return bad(format!("unknown argument {name:?}"));
                }
                None => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "pod": {"type": "string", "minLength": 1},
                "tail": {"type": "integer", "minimum": 1, "maximum": 10},
                "mode": {"enum": ["a", "b"]},
                "tags": {"type": "array", "items": {"type": "string"}},
                "on": {"type": "boolean"}
            },
            "required": ["pod"],
            "additionalProperties": false
        })
    }

    #[test]
    fn a_fitting_object_passes() {
        validate_args(
            &schema(),
            &json!({"pod": "web", "tail": 5, "mode": "a", "tags": ["x"], "on": true}),
        )
        .unwrap();
    }

    #[test]
    fn each_kind_of_misfit_is_named() {
        for (args, needle) in [
            (json!([]), "expected object"),
            (json!({}), "missing required argument \"pod\""),
            (json!({"pod": 3}), "arguments.pod: expected string"),
            (json!({"pod": ""}), "at least 1 characters"),
            (
                json!({"pod": "w", "tail": 0}),
                "arguments.tail: must be at least 1",
            ),
            (json!({"pod": "w", "tail": 11}), "must be at most 10"),
            (json!({"pod": "w", "tail": 1.5}), "expected integer"),
            (json!({"pod": "w", "mode": "c"}), "expected one of"),
            (
                json!({"pod": "w", "tags": [1]}),
                "arguments.tags[0]: expected string",
            ),
            (
                json!({"pod": "w", "color": "red"}),
                "unknown argument \"color\"",
            ),
        ] {
            let err = validate_args(&schema(), &args).unwrap_err();
            assert!(err.message().contains(needle), "{args}: {}", err.message());
        }
    }
}
