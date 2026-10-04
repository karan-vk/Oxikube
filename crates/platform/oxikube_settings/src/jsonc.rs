//! JSON-with-comments parsing and the layer merge.
//!
//! Settings files are JSONC: `//` and `/* */` comments and trailing commas are allowed
//! (`serde_json_lenient`, the parser Zed uses). Parsed layers are plain [`Value`]s; the store
//! merges them with [`merge_layer`] before any setting is deserialised, so a setting's
//! content struct sees one object with every layer applied.

use serde::Deserialize as _;
use serde_json::{Map, Value};

/// Parse JSONC text into a JSON object.
///
/// Blank text is an empty object (a new or truncated `settings.json` means "no overrides").
/// Errors carry the parser's line and column; trailing content after the object is an error.
pub fn parse_jsonc_object(text: &str) -> Result<Map<String, Value>, String> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let mut deserializer = serde_json_lenient::Deserializer::from_str(text);
    let value = Value::deserialize(&mut deserializer).map_err(|err| err.to_string())?;
    deserializer.end().map_err(|err| err.to_string())?;
    match value {
        Value::Object(map) => Ok(map),
        other => Err(format!(
            "settings must be a JSON object, found {}",
            value_kind(&other)
        )),
    }
}

/// Merge `overlay` into `base`: objects merge key by key (recursively); any other value in
/// the overlay replaces the base value, arrays included. `null` in the overlay is skipped, so
/// writing `null` in a higher layer means "use the layer below".
pub fn merge_layer(base: &mut Map<String, Value>, overlay: &Map<String, Value>) {
    for (key, value) in overlay {
        match (base.get_mut(key), value) {
            (_, Value::Null) => {}
            (Some(Value::Object(base_object)), Value::Object(overlay_object)) => {
                merge_layer(base_object, overlay_object);
            }
            _ => {
                base.insert(key.clone(), strip_nulls(value));
            }
        }
    }
}

/// `value` with `null` object members removed at every depth (array elements are kept).
pub fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_nulls).collect()),
        other => other.clone(),
    }
}

/// A short name for the JSON type of `value`, for error messages.
pub fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn parses_comments_and_trailing_commas() {
        let text = "// header\n{\n  /* block */ \"a\": 1, // tail\n  \"b\": [1, 2,],\n}\n";
        assert_eq!(
            parse_jsonc_object(text).unwrap(),
            object(json!({"a": 1, "b": [1, 2]}))
        );
    }

    /// Long floats parse to the nearest `f64` (the `float_roundtrip` parser), so a value the
    /// store wrote reads back bit for bit.
    #[test]
    fn parses_long_floats_exactly() {
        let value = 10.957_860_598_549_463_f64;
        let parsed = parse_jsonc_object(&format!("{{\"x\": {value}}}")).unwrap();
        assert_eq!(
            parsed["x"].as_f64().map(f64::to_bits),
            Some(value.to_bits())
        );
    }

    #[test]
    fn blank_text_is_an_empty_object() {
        assert!(parse_jsonc_object("  \n").unwrap().is_empty());
    }

    #[test]
    fn rejects_non_objects_and_trailing_content() {
        let err = parse_jsonc_object("[1]").unwrap_err();
        assert!(err.contains("an array"), "{err}");
        assert!(parse_jsonc_object("{} {}").is_err());
        let err = parse_jsonc_object("{\"a\": }").unwrap_err();
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn merge_is_deep_for_objects_and_replaces_everything_else() {
        let mut base = object(json!({
            "a": {"x": 1, "y": 2},
            "list": [1, 2, 3],
            "keep": true,
        }));
        let overlay = object(json!({
            "a": {"y": 20, "z": {"deep": null, "n": 1}},
            "list": [9],
            "keep": null,
        }));
        merge_layer(&mut base, &overlay);
        assert_eq!(
            Value::Object(base),
            json!({
                "a": {"x": 1, "y": 20, "z": {"n": 1}},
                "list": [9],
                "keep": true,
            })
        );
    }
}
