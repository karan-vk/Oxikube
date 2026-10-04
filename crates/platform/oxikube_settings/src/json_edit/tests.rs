// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/settings_json/src/settings_json.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Tests for the vendored JSON text edits. `object_replace`, `object_replace_escapes_new_key`,
//! `object_remove_and_rename_find_an_escaped_key_by_its_own_range` and
//! `test_infer_json_indent_size` are Zed's; the `update_*` tests below them are ours and drive
//! [`update_value_in_json_text`] the way the settings store does.

use super::*;
use serde_json::{Value, json};

/// Minimal stand-in for the `unindent` crate used by the upstream tests: strip the common
/// leading spaces of every line after the first, ignoring whitespace-only lines.
trait UnindentText {
    fn unindent_text(&self) -> String;
}

impl UnindentText for str {
    fn unindent_text(&self) -> String {
        let lines: Vec<&str> = self.split('\n').collect();
        let spaces = lines
            .iter()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.len() - line.trim_start_matches(' ').len())
            .min()
            .unwrap_or(0);
        let out: Vec<&str> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    line
                } else if line.len() > spaces {
                    &line[spaces..]
                } else {
                    ""
                }
            })
            .collect();
        let joined = out.join("\n");
        match joined.strip_prefix('\n') {
            Some(rest) => rest.to_owned(),
            None => joined,
        }
    }
}

#[test]
fn object_replace() {
    #[track_caller]
    fn check_object_replace(
        input: String,
        key_path: &[&str],
        value: Option<Value>,
        expected: String,
    ) {
        let result = replace_value_in_json_text(&input, key_path, 4, value.as_ref(), None);
        let mut result_str = input;
        result_str.replace_range(result.0, &result.1);
        assert_eq!(expected, result_str);
    }
    check_object_replace(
        r#"{
                "a": 1,
                "b": 2
            }"#
        .unindent_text(),
        &["b"],
        Some(json!(3)),
        r#"{
                "a": 1,
                "b": 3
            }"#
        .unindent_text(),
    );
    check_object_replace(
        r#"{
                "a": 1,
                "b": 2
            }"#
        .unindent_text(),
        &["b"],
        None,
        r#"{
                "a": 1
            }"#
        .unindent_text(),
    );
    check_object_replace(
        r#"{
                "a": 1,
                "b": 2
            }"#
        .unindent_text(),
        &["c"],
        Some(json!(3)),
        r#"{
                "c": 3,
                "a": 1,
                "b": 2
            }"#
        .unindent_text(),
    );
    check_object_replace(
        r#"{
                "a": 1,
                "b": {
                    "c": 2,
                    "d": 3,
                }
            }"#
        .unindent_text(),
        &["b", "c"],
        Some(json!([1, 2, 3])),
        r#"{
                "a": 1,
                "b": {
                    "c": [
                        1,
                        2,
                        3
                    ],
                    "d": 3,
                }
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "name": "old_name",
                "id": 123
            }"#
        .unindent_text(),
        &["name"],
        Some(json!("new_name")),
        r#"{
                "name": "new_name",
                "id": 123
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "enabled": false,
                "count": 5
            }"#
        .unindent_text(),
        &["enabled"],
        Some(json!(true)),
        r#"{
                "enabled": true,
                "count": 5
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "value": null,
                "other": "test"
            }"#
        .unindent_text(),
        &["value"],
        Some(json!(42)),
        r#"{
                "value": 42,
                "other": "test"
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "config": {
                    "old": true
                },
                "name": "test"
            }"#
        .unindent_text(),
        &["config"],
        Some(json!({"new": false, "count": 3})),
        r#"{
                "config": {
                    "new": false,
                    "count": 3
                },
                "name": "test"
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                // This is a comment
                "a": 1,
                "b": 2 // Another comment
            }"#
        .unindent_text(),
        &["b"],
        Some(json!({"foo": "bar"})),
        r#"{
                // This is a comment
                "a": 1,
                "b": {
                    "foo": "bar"
                } // Another comment
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{}"#.to_string(),
        &["new_key"],
        Some(json!("value")),
        r#"{
                "new_key": "value"
            }
            "#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "only_key": 123
            }"#
        .unindent_text(),
        &["only_key"],
        None,
        "{\n    \n}".to_string(),
    );

    check_object_replace(
        r#"{
                "level1": {
                    "level2": {
                        "level3": {
                            "target": "old"
                        }
                    }
                }
            }"#
        .unindent_text(),
        &["level1", "level2", "level3", "target"],
        Some(json!("new")),
        r#"{
                "level1": {
                    "level2": {
                        "level3": {
                            "target": "new"
                        }
                    }
                }
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "parent": {}
            }"#
        .unindent_text(),
        &["parent", "child"],
        Some(json!("value")),
        r#"{
                "parent": {
                    "child": "value"
                }
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "a": 1,
                "b": 2,
            }"#
        .unindent_text(),
        &["b"],
        Some(json!(3)),
        r#"{
                "a": 1,
                "b": 3,
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "items": [1, 2, 3],
                "count": 3
            }"#
        .unindent_text(),
        &["items", "1"],
        Some(json!(5)),
        r#"{
                "items": {
                    "1": 5
                },
                "count": 3
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "items": [1, 2, 3],
                "count": 3
            }"#
        .unindent_text(),
        &["items", "1"],
        None,
        r#"{
                "items": {
                    "1": null
                },
                "count": 3
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "items": [1, 2, 3],
                "count": 3
            }"#
        .unindent_text(),
        &["items"],
        Some(json!(["a", "b", "c", "d"])),
        r#"{
                "items": [
                    "a",
                    "b",
                    "c",
                    "d"
                ],
                "count": 3
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                "0": "zero",
                "1": "one"
            }"#
        .unindent_text(),
        &["1"],
        Some(json!("ONE")),
        r#"{
                "0": "zero",
                "1": "ONE"
            }"#
        .unindent_text(),
    );
    // Test with comments between object members
    check_object_replace(
        r#"{
                "a": 1,
                // Comment between members
                "b": 2,
                /* Block comment */
                "c": 3
            }"#
        .unindent_text(),
        &["b"],
        Some(json!({"nested": true})),
        r#"{
                "a": 1,
                // Comment between members
                "b": {
                    "nested": true
                },
                /* Block comment */
                "c": 3
            }"#
        .unindent_text(),
    );

    // Test with trailing comments on replaced value
    check_object_replace(
        r#"{
                "a": 1, // keep this comment
                "b": 2  // this should stay
            }"#
        .unindent_text(),
        &["a"],
        Some(json!("changed")),
        r#"{
                "a": "changed", // keep this comment
                "b": 2  // this should stay
            }"#
        .unindent_text(),
    );

    // Test with deep indentation
    check_object_replace(
        r#"{
                        "deeply": {
                                "nested": {
                                        "value": "old"
                                }
                        }
                }"#
        .unindent_text(),
        &["deeply", "nested", "value"],
        Some(json!("new")),
        r#"{
                        "deeply": {
                                "nested": {
                                        "value": "new"
                                }
                        }
                }"#
        .unindent_text(),
    );

    // Test removing value with comment preservation
    check_object_replace(
        r#"{
                // Header comment
                "a": 1,
                // This comment belongs to b
                "b": 2,
                // This comment belongs to c
                "c": 3
            }"#
        .unindent_text(),
        &["b"],
        None,
        r#"{
                // Header comment
                "a": 1,
                // This comment belongs to b
                // This comment belongs to c
                "c": 3
            }"#
        .unindent_text(),
    );

    // Test with multiline block comments
    check_object_replace(
        r#"{
                /*
                 * This is a multiline
                 * block comment
                 */
                "value": "old",
                /* Another block */ "other": 123
            }"#
        .unindent_text(),
        &["value"],
        Some(json!("new")),
        r#"{
                /*
                 * This is a multiline
                 * block comment
                 */
                "value": "new",
                /* Another block */ "other": 123
            }"#
        .unindent_text(),
    );

    check_object_replace(
        r#"{
                // This object is empty
            }"#
        .unindent_text(),
        &["key"],
        Some(json!("value")),
        r#"{
                // This object is empty
                "key": "value"
            }
            "#
        .unindent_text(),
    );

    // Test replacing in object with only comments
    check_object_replace(
        r#"{
                // Comment 1
                // Comment 2
            }"#
        .unindent_text(),
        &["new"],
        Some(json!(42)),
        r#"{
                // Comment 1
                // Comment 2
                "new": 42
            }
            "#
        .unindent_text(),
    );

    // Test with inconsistent spacing
    check_object_replace(
        r#"{
              "a":1,
                    "b"  :  2  ,
                "c":   3
            }"#
        .unindent_text(),
        &["b"],
        Some(json!("spaced")),
        r#"{
              "a":1,
                    "b"  :  "spaced"  ,
                "c":   3
            }"#
        .unindent_text(),
    );
}

#[test]
fn object_replace_escapes_new_key() {
    // An object that already has a key: an empty one takes the nested-construction path.
    let single_line = r#"{"theme": "One Dark"}"#;
    let multi_line = "{\n    \"theme\": \"One Dark\"\n}";
    let key = r#"/home/me/say "hi" C:\x"#;

    for input in [single_line, multi_line] {
        let mut text = input.to_string();
        let (range, replacement) =
            replace_value_in_json_text(&text, &[key], 4, Some(&json!("One Light")), None);
        text.replace_range(range, &replacement);

        let parsed: Value = serde_json::from_str(&text)
            .expect("a folder name carrying a quote must not break settings.json");
        assert_eq!(parsed, json!({ "theme": "One Dark", key: "One Light" }));
    }
}

#[test]
fn object_remove_and_rename_find_an_escaped_key_by_its_own_range() {
    let key = "say \"hi\"";
    let input = format!(
        "{{{}: \"V\", \"theme\": \"One Dark\"}}",
        serde_json::to_string(key).unwrap()
    );

    let mut removed = input.clone();
    let (range, replacement) = replace_value_in_json_text(&removed, &[key], 4, None, None);
    removed.replace_range(range, &replacement);
    let parsed: Value = serde_json::from_str(&removed).expect("removal must leave valid JSON");
    assert_eq!(parsed, json!({ "theme": "One Dark" }));

    let mut renamed = input;
    let (range, replacement) =
        replace_value_in_json_text(&renamed, &[key], 4, Some(&json!("V2")), Some("plain"));
    renamed.replace_range(range, &replacement);
    let parsed: Value = serde_json::from_str(&renamed).expect("rename must leave valid JSON");
    assert_eq!(parsed, json!({ "plain": "V2", "theme": "One Dark" }));
}

#[test]
fn test_infer_json_indent_size() {
    let json_2_spaces = r#"{
  "key1": "value1",
  "nested": {
    "key2": "value2",
    "array": [
      1,
      2,
      3
    ]
  }
}"#;
    assert_eq!(infer_json_indent_size(json_2_spaces), 2);

    let json_4_spaces = r#"{
    "key1": "value1",
    "nested": {
        "key2": "value2",
        "array": [
            1,
            2,
            3
        ]
    }
}"#;
    assert_eq!(infer_json_indent_size(json_4_spaces), 4);

    let json_8_spaces = r#"{
        "key1": "value1",
        "nested": {
                "key2": "value2"
        }
}"#;
    assert_eq!(infer_json_indent_size(json_8_spaces), 8);

    let json_single_line = r#"{"key": "value", "nested": {"inner": "data"}}"#;
    assert_eq!(infer_json_indent_size(json_single_line), 2);

    let json_empty = r#"{}"#;
    assert_eq!(infer_json_indent_size(json_empty), 2);

    let json_array = r#"[
  {
    "id": 1,
    "name": "first"
  },
  {
    "id": 2,
    "name": "second"
  }
]"#;
    assert_eq!(infer_json_indent_size(json_array), 2);

    let json_mixed = r#"{
  "a": {
    "b": {
        "c": "value"
    }
  },
  "d": "value2"
}"#;
    assert_eq!(infer_json_indent_size(json_mixed), 2);
}
