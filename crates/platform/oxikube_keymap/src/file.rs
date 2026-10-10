//! The `keymap.json` format (Zed's): a JSON-with-comments list of sections.
//!
//! ```json
//! [
//!   {
//!     "context": "Workspace",
//!     "use_key_equivalents": true,
//!     "bindings": {
//!       "cmd-shift-w": "workspace::CloseActiveItem",
//!       "cmd-k": ["palette::Open", { "query": "pod" }],
//!       "ctrl-x": null
//!     }
//!   }
//! ]
//! ```
//!
//! A binding's value is an action name, `[name, data]` for an action with data, or `null` to
//! unbind the key. Sections apply in file order and so do the bindings inside one (the later
//! binding wins), which is why `bindings` stays an ordered map.
//!
//! Parsing is lenient where it can be: the file must be a JSON list, but one bad section is
//! reported and skipped and the rest still load.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::diagnostics::{KeymapDiagnostic, KeymapProblem};
use crate::layer::KeymapLayer;
use crate::lines::SourceLines;

/// One entry of the keymap list: bindings that share a context.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeymapSection {
    /// Key-context expression (`Workspace`, `ResourceTable && !Editing`) the bindings are active in;
    /// absent or empty means everywhere.
    #[serde(default)]
    pub context: Option<String>,
    /// Make shortcuts follow the key's character on non-US layouts instead of its position.
    #[serde(default)]
    pub use_key_equivalents: bool,
    /// Keystrokes (`ctrl-k ctrl-s` is a two-keystroke sequence) to what they do, in file order.
    #[serde(default)]
    pub bindings: Map<String, Value>,
}

impl KeymapSection {
    /// The context expression, `None` when absent or blank.
    pub fn context_expr(&self) -> Option<&str> {
        self.context
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
    }
}

/// What a binding does.
#[derive(Clone, Debug, PartialEq)]
pub enum KeymapAction {
    /// `null`: remove the binding of this key (from the same or a lower layer).
    Unbind,
    /// Dispatch the named action, with data for actions that take it.
    Action {
        /// `namespace::Name`.
        name: String,
        /// The JSON data of `[name, data]`.
        data: Option<Value>,
    },
}

impl KeymapAction {
    fn named(name: &str, data: Option<Value>) -> Self {
        Self::Action {
            name: name.to_owned(),
            data,
        }
    }

    /// Read a binding value: `null`, `"name"` or `["name"]` / `["name", data]`.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        match value {
            Value::Null => Ok(Self::Unbind),
            Value::String(name) => Ok(Self::named(name, None)),
            Value::Array(items) => match items.as_slice() {
                [Value::String(name)] => Ok(Self::named(name, None)),
                [Value::String(name), data] => Ok(Self::named(name, Some(data.clone()))),
                _ => Err("expected [\"namespace::Name\", data]".to_owned()),
            },
            other => Err(format!(
                "expected an action name, [name, data] or null, found {}",
                json_kind(other)
            )),
        }
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The sections of one layer's file, plus what went wrong reading it.
#[derive(Clone, Debug, Default)]
pub struct ParsedKeymap {
    /// The sections that parsed, in file order. Each keeps its index in the file for reports.
    pub sections: Vec<(usize, KeymapSection)>,
    /// Problems with individual sections, with their lines.
    pub diagnostics: Vec<KeymapDiagnostic>,
    /// Where each section, context and binding is in the text, to locate later problems.
    pub lines: SourceLines,
}

/// Parse a keymap file. `Err` is a whole-file problem (not JSON, not a list): the caller keeps
/// the layer's previous sections. Blank text is an empty keymap.
pub fn parse_keymap(text: &str, layer: KeymapLayer) -> Result<ParsedKeymap, KeymapDiagnostic> {
    if text.trim().is_empty() {
        return Ok(ParsedKeymap::default());
    }
    let mut deserializer = serde_json_lenient::Deserializer::from_str(text);
    let syntax = |err: serde_json_lenient::Error| {
        KeymapDiagnostic::file(layer, Some(err.line()), err.to_string())
    };
    let value = Value::deserialize(&mut deserializer).map_err(syntax)?;
    deserializer.end().map_err(syntax)?;
    let Value::Array(entries) = value else {
        return Err(KeymapDiagnostic::file(
            layer,
            None,
            format!(
                "the keymap must be a list of sections, found {}",
                json_kind(&value)
            ),
        ));
    };

    let mut parsed = ParsedKeymap {
        lines: SourceLines::scan(text),
        ..ParsedKeymap::default()
    };
    for (index, entry) in entries.into_iter().enumerate() {
        match serde_json::from_value::<KeymapSection>(entry) {
            Ok(section) => parsed.sections.push((index, section)),
            Err(err) => {
                let mut diagnostic = KeymapDiagnostic::section(
                    layer,
                    index,
                    KeymapProblem::InvalidSection {
                        message: err.to_string(),
                    },
                );
                diagnostic.locate(&parsed.lines);
                parsed.diagnostics.push(diagnostic);
            }
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_comments_trailing_commas_and_all_binding_shapes() {
        let parsed = parse_keymap(
            r#"
            // my keys
            [
              {
                "context": "Workspace",
                "use_key_equivalents": true,
                "bindings": {
                  "cmd-a": "a::One",
                  "cmd-b": ["b::Two", {"n": 1}],
                  "cmd-c": null, /* gone */
                },
              },
            ]"#,
            KeymapLayer::User,
        )
        .unwrap();
        assert!(parsed.diagnostics.is_empty());
        let (index, section) = &parsed.sections[0];
        assert_eq!(*index, 0);
        assert_eq!(section.context_expr(), Some("Workspace"));
        assert!(section.use_key_equivalents);
        let keys: Vec<_> = section.bindings.keys().map(String::as_str).collect();
        assert_eq!(keys, ["cmd-a", "cmd-b", "cmd-c"], "file order is kept");
        assert_eq!(
            KeymapAction::from_json(&section.bindings["cmd-b"]),
            Ok(KeymapAction::Action {
                name: "b::Two".into(),
                data: Some(json!({"n": 1}))
            })
        );
        assert_eq!(
            KeymapAction::from_json(&section.bindings["cmd-c"]),
            Ok(KeymapAction::Unbind)
        );
    }

    #[test]
    fn blank_text_is_an_empty_keymap() {
        let parsed = parse_keymap("  \n", KeymapLayer::User).unwrap();
        assert!(parsed.sections.is_empty() && parsed.diagnostics.is_empty());
    }

    #[test]
    fn whole_file_problems_are_errors() {
        for text in ["[{", "{}", "[] []", "42"] {
            let err = parse_keymap(text, KeymapLayer::User).unwrap_err();
            assert!(
                matches!(err.problem, KeymapProblem::InvalidFile { .. }),
                "{text}"
            );
        }
    }

    #[test]
    fn a_bad_section_is_reported_and_the_rest_load() {
        let parsed = parse_keymap(
            r#"[{"bindings": {"a": "x::Y"}}, 7, {"bindngs": {}}, {"bindings": {"b": "x::Z"}}]"#,
            KeymapLayer::User,
        )
        .unwrap();
        let indices: Vec<_> = parsed.sections.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, [0, 3]);
        assert_eq!(parsed.diagnostics.len(), 2);
        assert_eq!(parsed.diagnostics[0].section, Some(1));
        assert_eq!(parsed.diagnostics[1].section, Some(2));
        assert_eq!(
            parsed.diagnostics[0].line,
            Some(1),
            "one line: every section is on it"
        );
    }

    #[test]
    fn problems_carry_their_line() {
        let parsed = parse_keymap(
            "[\n  {\"bindings\": {}},\n  7,\n  {\"bindngs\": {}}\n]",
            KeymapLayer::User,
        )
        .unwrap();
        let lines: Vec<_> = parsed.diagnostics.iter().map(|d| d.line).collect();
        assert_eq!(lines, [Some(3), Some(4)]);
        let err = parse_keymap("[\n  {\n    \"bindings\": {\n", KeymapLayer::User).unwrap_err();
        assert!(err.line.is_some_and(|line| line >= 3), "{err}");
        let err = parse_keymap("{}", KeymapLayer::User).unwrap_err();
        assert_eq!(err.line, None);
    }

    #[test]
    fn rejects_malformed_binding_values() {
        for value in [
            json!(3),
            json!([]),
            json!(["a", 1, 2]),
            json!([1]),
            json!({"a": 1}),
        ] {
            assert!(KeymapAction::from_json(&value).is_err(), "{value}");
        }
        assert!(KeymapAction::from_json(&json!(["a::B"])).is_ok());
    }
}
