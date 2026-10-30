// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/settings_json/src/settings_json.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Zed's array tests (`object_replace_array`, `array_replace`, `array_append`), unchanged
//! apart from the `unindent` stand-in and `pretty_assertions` -> `assert_eq!`; the
//! Oxikube cases at the end cover comments, CRLF and tabs in `keymap.json`-shaped documents.

use super::tests::UnindentText as _;
use super::*;
use serde_json::{Value, json};

#[test]
fn object_replace_array() {
    // Tests replacing values within arrays that are nested inside objects.
    // Uses "#N" syntax in key paths to indicate array indices.
    #[track_caller]
    fn check_object_replace_array(
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

    // Basic array element replacement
    check_object_replace_array(
        r#"{
            "a": [1, 3],
        }"#
        .unindent_text(),
        &["a", "#1"],
        Some(json!(2)),
        r#"{
            "a": [1, 2],
        }"#
        .unindent_text(),
    );

    // Replace first element
    check_object_replace_array(
        r#"{
            "items": [1, 2, 3]
        }"#
        .unindent_text(),
        &["items", "#0"],
        Some(json!(10)),
        r#"{
            "items": [10, 2, 3]
        }"#
        .unindent_text(),
    );

    // Replace last element
    check_object_replace_array(
        r#"{
            "items": [1, 2, 3]
        }"#
        .unindent_text(),
        &["items", "#2"],
        Some(json!(30)),
        r#"{
            "items": [1, 2, 30]
        }"#
        .unindent_text(),
    );

    // Replace string in array
    check_object_replace_array(
        r#"{
            "names": ["alice", "bob", "charlie"]
        }"#
        .unindent_text(),
        &["names", "#1"],
        Some(json!("robert")),
        r#"{
            "names": ["alice", "robert", "charlie"]
        }"#
        .unindent_text(),
    );

    // Replace boolean
    check_object_replace_array(
        r#"{
            "flags": [true, false, true]
        }"#
        .unindent_text(),
        &["flags", "#0"],
        Some(json!(false)),
        r#"{
            "flags": [false, false, true]
        }"#
        .unindent_text(),
    );

    // Replace null with value
    check_object_replace_array(
        r#"{
            "values": [null, 2, null]
        }"#
        .unindent_text(),
        &["values", "#0"],
        Some(json!(1)),
        r#"{
            "values": [1, 2, null]
        }"#
        .unindent_text(),
    );

    // Replace value with null
    check_object_replace_array(
        r#"{
            "data": [1, 2, 3]
        }"#
        .unindent_text(),
        &["data", "#1"],
        Some(json!(null)),
        r#"{
            "data": [1, null, 3]
        }"#
        .unindent_text(),
    );

    // Replace simple value with object
    check_object_replace_array(
        r#"{
            "list": [1, 2, 3]
        }"#
        .unindent_text(),
        &["list", "#1"],
        Some(json!({"value": 2, "label": "two"})),
        r#"{
            "list": [1, { "value": 2, "label": "two" }, 3]
        }"#
        .unindent_text(),
    );

    // Replace simple value with nested array
    check_object_replace_array(
        r#"{
            "matrix": [1, 2, 3]
        }"#
        .unindent_text(),
        &["matrix", "#1"],
        Some(json!([20, 21, 22])),
        r#"{
            "matrix": [1, [ 20, 21, 22 ], 3]
        }"#
        .unindent_text(),
    );

    // Replace object in array
    check_object_replace_array(
        r#"{
            "users": [
                {"name": "alice"},
                {"name": "bob"},
                {"name": "charlie"}
            ]
        }"#
        .unindent_text(),
        &["users", "#1"],
        Some(json!({"name": "robert", "age": 30})),
        r#"{
            "users": [
                {"name": "alice"},
                { "name": "robert", "age": 30 },
                {"name": "charlie"}
            ]
        }"#
        .unindent_text(),
    );

    // Replace property within object in array
    check_object_replace_array(
        r#"{
            "users": [
                {"name": "alice", "age": 25},
                {"name": "bob", "age": 30},
                {"name": "charlie", "age": 35}
            ]
        }"#
        .unindent_text(),
        &["users", "#1", "age"],
        Some(json!(31)),
        r#"{
            "users": [
                {"name": "alice", "age": 25},
                {"name": "bob", "age": 31},
                {"name": "charlie", "age": 35}
            ]
        }"#
        .unindent_text(),
    );

    // Add new property to object in array
    check_object_replace_array(
        r#"{
            "items": [
                {"id": 1},
                {"id": 2},
                {"id": 3}
            ]
        }"#
        .unindent_text(),
        &["items", "#1", "name"],
        Some(json!("Item Two")),
        r#"{
            "items": [
                {"id": 1},
                {"name": "Item Two", "id": 2},
                {"id": 3}
            ]
        }"#
        .unindent_text(),
    );

    // Remove property from object in array
    check_object_replace_array(
        r#"{
            "items": [
                {"id": 1, "name": "one"},
                {"id": 2, "name": "two"},
                {"id": 3, "name": "three"}
            ]
        }"#
        .unindent_text(),
        &["items", "#1", "name"],
        None,
        r#"{
            "items": [
                {"id": 1, "name": "one"},
                {"id": 2},
                {"id": 3, "name": "three"}
            ]
        }"#
        .unindent_text(),
    );

    // Deeply nested: array in object in array
    check_object_replace_array(
        r#"{
            "data": [
                {
                    "values": [1, 2, 3]
                },
                {
                    "values": [4, 5, 6]
                }
            ]
        }"#
        .unindent_text(),
        &["data", "#0", "values", "#1"],
        Some(json!(20)),
        r#"{
            "data": [
                {
                    "values": [1, 20, 3]
                },
                {
                    "values": [4, 5, 6]
                }
            ]
        }"#
        .unindent_text(),
    );

    // Multiple levels of nesting
    check_object_replace_array(
        r#"{
            "root": {
                "level1": [
                    {
                        "level2": {
                            "level3": [10, 20, 30]
                        }
                    }
                ]
            }
        }"#
        .unindent_text(),
        &["root", "level1", "#0", "level2", "level3", "#2"],
        Some(json!(300)),
        r#"{
            "root": {
                "level1": [
                    {
                        "level2": {
                            "level3": [10, 20, 300]
                        }
                    }
                ]
            }
        }"#
        .unindent_text(),
    );

    // Array with mixed types
    check_object_replace_array(
        r#"{
            "mixed": [1, "two", true, null, {"five": 5}]
        }"#
        .unindent_text(),
        &["mixed", "#3"],
        Some(json!({"four": 4})),
        r#"{
            "mixed": [1, "two", true, { "four": 4 }, {"five": 5}]
        }"#
        .unindent_text(),
    );

    // Replace with complex object
    check_object_replace_array(
        r#"{
            "config": [
                "simple",
                "values"
            ]
        }"#
        .unindent_text(),
        &["config", "#0"],
        Some(json!({
            "type": "complex",
            "settings": {
                "enabled": true,
                "level": 5
            }
        })),
        r#"{
            "config": [
                {
                    "type": "complex",
                    "settings": {
                        "enabled": true,
                        "level": 5
                    }
                },
                "values"
            ]
        }"#
        .unindent_text(),
    );

    // Array with trailing comma
    check_object_replace_array(
        r#"{
            "items": [
                1,
                2,
                3,
            ]
        }"#
        .unindent_text(),
        &["items", "#1"],
        Some(json!(20)),
        r#"{
            "items": [
                1,
                20,
                3,
            ]
        }"#
        .unindent_text(),
    );

    // Array with comments
    check_object_replace_array(
        r#"{
            "items": [
                1, // first item
                2, // second item
                3  // third item
            ]
        }"#
        .unindent_text(),
        &["items", "#1"],
        Some(json!(20)),
        r#"{
            "items": [
                1, // first item
                20, // second item
                3  // third item
            ]
        }"#
        .unindent_text(),
    );

    // Multiple arrays in object
    check_object_replace_array(
        r#"{
            "first": [1, 2, 3],
            "second": [4, 5, 6],
            "third": [7, 8, 9]
        }"#
        .unindent_text(),
        &["second", "#1"],
        Some(json!(50)),
        r#"{
            "first": [1, 2, 3],
            "second": [4, 50, 6],
            "third": [7, 8, 9]
        }"#
        .unindent_text(),
    );

    // Empty array - add first element
    check_object_replace_array(
        r#"{
            "empty": []
        }"#
        .unindent_text(),
        &["empty", "#0"],
        Some(json!("first")),
        r#"{
            "empty": ["first"]
        }"#
        .unindent_text(),
    );

    // Array of arrays
    check_object_replace_array(
        r#"{
            "matrix": [
                [1, 2],
                [3, 4],
                [5, 6]
            ]
        }"#
        .unindent_text(),
        &["matrix", "#1", "#0"],
        Some(json!(30)),
        r#"{
            "matrix": [
                [1, 2],
                [30, 4],
                [5, 6]
            ]
        }"#
        .unindent_text(),
    );

    // Replace nested object property in array element
    check_object_replace_array(
        r#"{
            "users": [
                {
                    "name": "alice",
                    "address": {
                        "city": "NYC",
                        "zip": "10001"
                    }
                }
            ]
        }"#
        .unindent_text(),
        &["users", "#0", "address", "city"],
        Some(json!("Boston")),
        r#"{
            "users": [
                {
                    "name": "alice",
                    "address": {
                        "city": "Boston",
                        "zip": "10001"
                    }
                }
            ]
        }"#
        .unindent_text(),
    );

    // Add element past end of array
    check_object_replace_array(
        r#"{
            "items": [1, 2]
        }"#
        .unindent_text(),
        &["items", "#5"],
        Some(json!(6)),
        r#"{
            "items": [1, 2, 6]
        }"#
        .unindent_text(),
    );

    // Complex nested structure
    check_object_replace_array(
        r#"{
            "app": {
                "modules": [
                    {
                        "name": "auth",
                        "routes": [
                            {"path": "/login", "method": "POST"},
                            {"path": "/logout", "method": "POST"}
                        ]
                    },
                    {
                        "name": "api",
                        "routes": [
                            {"path": "/users", "method": "GET"},
                            {"path": "/users", "method": "POST"}
                        ]
                    }
                ]
            }
        }"#
        .unindent_text(),
        &["app", "modules", "#1", "routes", "#0", "method"],
        Some(json!("PUT")),
        r#"{
            "app": {
                "modules": [
                    {
                        "name": "auth",
                        "routes": [
                            {"path": "/login", "method": "POST"},
                            {"path": "/logout", "method": "POST"}
                        ]
                    },
                    {
                        "name": "api",
                        "routes": [
                            {"path": "/users", "method": "PUT"},
                            {"path": "/users", "method": "POST"}
                        ]
                    }
                ]
            }
        }"#
        .unindent_text(),
    );

    // Escaped strings in array
    check_object_replace_array(
        r#"{
            "messages": ["hello", "world"]
        }"#
        .unindent_text(),
        &["messages", "#0"],
        Some(json!("hello \"quoted\" world")),
        r#"{
            "messages": ["hello \"quoted\" world", "world"]
        }"#
        .unindent_text(),
    );

    // Block comments
    check_object_replace_array(
        r#"{
            "data": [
                /* first */ 1,
                /* second */ 2,
                /* third */ 3
            ]
        }"#
        .unindent_text(),
        &["data", "#1"],
        Some(json!(20)),
        r#"{
            "data": [
                /* first */ 1,
                /* second */ 20,
                /* third */ 3
            ]
        }"#
        .unindent_text(),
    );

    // Inline array
    check_object_replace_array(
        r#"{"items": [1, 2, 3], "count": 3}"#.to_string(),
        &["items", "#1"],
        Some(json!(20)),
        r#"{"items": [1, 20, 3], "count": 3}"#.to_string(),
    );

    // Single element array
    check_object_replace_array(
        r#"{
            "single": [42]
        }"#
        .unindent_text(),
        &["single", "#0"],
        Some(json!(100)),
        r#"{
            "single": [100]
        }"#
        .unindent_text(),
    );

    // Inconsistent formatting
    check_object_replace_array(
        r#"{
            "messy": [1,
                2,
                    3,
            4]
        }"#
        .unindent_text(),
        &["messy", "#2"],
        Some(json!(30)),
        r#"{
            "messy": [1,
                2,
                    30,
            4]
        }"#
        .unindent_text(),
    );

    // Creates array if has numbered key
    check_object_replace_array(
        r#"{
            "array": {"foo": "bar"}
        }"#
        .unindent_text(),
        &["array", "#3"],
        Some(json!(4)),
        r#"{
            "array": [
                4
            ]
        }"#
        .unindent_text(),
    );

    // Replace non-array element within array with array
    check_object_replace_array(
        r#"{
            "matrix": [
                [1, 2],
                [3, 4],
                [5, 6]
            ]
        }"#
        .unindent_text(),
        &["matrix", "#1", "#0"],
        Some(json!(["foo", "bar"])),
        r#"{
            "matrix": [
                [1, 2],
                [[ "foo", "bar" ], 4],
                [5, 6]
            ]
        }"#
        .unindent_text(),
    );
    // Replace non-array element within array with array
    check_object_replace_array(
        r#"{
            "matrix": [
                [1, 2],
                [3, 4],
                [5, 6]
            ]
        }"#
        .unindent_text(),
        &["matrix", "#1", "#0", "#3"],
        Some(json!(["foo", "bar"])),
        r#"{
            "matrix": [
                [1, 2],
                [[ [ "foo", "bar" ] ], 4],
                [5, 6]
            ]
        }"#
        .unindent_text(),
    );

    // Create array in key that doesn't exist
    check_object_replace_array(
        r#"{
            "foo": {}
        }"#
        .unindent_text(),
        &["foo", "bar", "#0"],
        Some(json!({"is_object": true})),
        r#"{
            "foo": {
                "bar": [
                    {
                        "is_object": true
                    }
                ]
            }
        }"#
        .unindent_text(),
    );
}

#[test]
fn array_replace() {
    #[track_caller]
    fn check_array_replace(
        input: impl ToString,
        index: usize,
        key_path: &[&str],
        value: Option<Value>,
        expected: impl ToString,
    ) {
        let input = input.to_string();
        let result = replace_top_level_array_value_in_json_text(
            &input,
            key_path,
            value.as_ref(),
            None,
            index,
            4,
        );
        let mut result_str = input;
        result_str.replace_range(result.0, &result.1);
        assert_eq!(expected.to_string(), result_str);
    }

    check_array_replace(r#"[1, 3, 3]"#, 1, &[], Some(json!(2)), r#"[1, 2, 3]"#);
    check_array_replace(r#"[1, 3, 3]"#, 2, &[], Some(json!(2)), r#"[1, 3, 2]"#);
    check_array_replace(r#"[1, 3, 3,]"#, 3, &[], Some(json!(2)), r#"[1, 3, 3, 2]"#);
    check_array_replace(r#"[1, 3, 3,]"#, 100, &[], Some(json!(2)), r#"[1, 3, 3, 2]"#);
    check_array_replace(
        r#"[
            1,
            2,
            3,
        ]"#
        .unindent_text(),
        1,
        &[],
        Some(json!({"foo": "bar", "baz": "qux"})),
        r#"[
            1,
            {
                "foo": "bar",
                "baz": "qux"
            },
            3,
        ]"#
        .unindent_text(),
    );
    check_array_replace(
        r#"[1, 3, 3,]"#,
        1,
        &[],
        Some(json!({"foo": "bar", "baz": "qux"})),
        r#"[1, { "foo": "bar", "baz": "qux" }, 3,]"#,
    );

    check_array_replace(
        r#"[1, { "foo": "bar", "baz": "qux" }, 3,]"#,
        1,
        &["baz"],
        Some(json!({"qux": "quz"})),
        r#"[1, { "foo": "bar", "baz": { "qux": "quz" } }, 3,]"#,
    );

    check_array_replace(
        r#"[
            1,
            {
                "foo": "bar",
                "baz": "qux"
            },
            3
        ]"#,
        1,
        &["baz"],
        Some(json!({"qux": "quz"})),
        r#"[
            1,
            {
                "foo": "bar",
                "baz": {
                    "qux": "quz"
                }
            },
            3
        ]"#,
    );

    check_array_replace(
        r#"[
            1,
            {
                "foo": "bar",
                "baz": {
                    "qux": "quz"
                }
            },
            3
        ]"#,
        1,
        &["baz"],
        Some(json!("qux")),
        r#"[
            1,
            {
                "foo": "bar",
                "baz": "qux"
            },
            3
        ]"#,
    );

    check_array_replace(
        r#"[
            1,
            {
                "foo": "bar",
                // some comment to keep
                "baz": {
                    // some comment to remove
                    "qux": "quz"
                }
                // some other comment to keep
            },
            3
        ]"#,
        1,
        &["baz"],
        Some(json!("qux")),
        r#"[
            1,
            {
                "foo": "bar",
                // some comment to keep
                "baz": "qux"
                // some other comment to keep
            },
            3
        ]"#,
    );

    // Test with comments between array elements
    check_array_replace(
        r#"[
            1,
            // This is element 2
            2,
            /* Block comment */ 3,
            4 // Trailing comment
        ]"#,
        2,
        &[],
        Some(json!("replaced")),
        r#"[
            1,
            // This is element 2
            2,
            /* Block comment */ "replaced",
            4 // Trailing comment
        ]"#,
    );

    // Test empty array with comments
    check_array_replace(
        r#"[
            // Empty array with comment
        ]"#
        .unindent_text(),
        0,
        &[],
        Some(json!("first")),
        r#"[
            // Empty array with comment
            "first"
        ]"#
        .unindent_text(),
    );
    check_array_replace(
        r#"[]"#.unindent_text(),
        0,
        &[],
        Some(json!("first")),
        r#"["first"]"#.unindent_text(),
    );

    // Test array with leading comments
    check_array_replace(
        r#"[
            // Leading comment
            // Another leading comment
            1,
            2
        ]"#,
        0,
        &[],
        Some(json!({"new": "object"})),
        r#"[
            // Leading comment
            // Another leading comment
            {
                "new": "object"
            },
            2
        ]"#,
    );

    // Test with deep indentation
    check_array_replace(
        r#"[
                    1,
                    2,
                    3
                ]"#,
        1,
        &[],
        Some(json!("deep")),
        r#"[
                    1,
                    "deep",
                    3
                ]"#,
    );

    // Test with mixed spacing
    check_array_replace(
        r#"[1,2,   3,    4]"#,
        2,
        &[],
        Some(json!("spaced")),
        r#"[1,2,   "spaced",    4]"#,
    );

    // Test replacing nested array element
    check_array_replace(
        r#"[
            [1, 2, 3],
            [4, 5, 6],
            [7, 8, 9]
        ]"#,
        1,
        &[],
        Some(json!(["a", "b", "c", "d"])),
        r#"[
            [1, 2, 3],
            [
                "a",
                "b",
                "c",
                "d"
            ],
            [7, 8, 9]
        ]"#,
    );

    // Test with multiline block comments
    check_array_replace(
        r#"[
            /*
             * This is a
             * multiline comment
             */
            "first",
            "second"
        ]"#,
        0,
        &[],
        Some(json!("updated")),
        r#"[
            /*
             * This is a
             * multiline comment
             */
            "updated",
            "second"
        ]"#,
    );

    // Test replacing with null
    check_array_replace(
        r#"[true, false, true]"#,
        1,
        &[],
        Some(json!(null)),
        r#"[true, null, true]"#,
    );

    // Test single element array
    check_array_replace(
        r#"[42]"#,
        0,
        &[],
        Some(json!({"answer": 42})),
        r#"[{ "answer": 42 }]"#,
    );

    // Test array with only comments
    check_array_replace(
        r#"[
            // Comment 1
            // Comment 2
            // Comment 3
        ]"#
        .unindent_text(),
        10,
        &[],
        Some(json!(123)),
        r#"[
            // Comment 1
            // Comment 2
            // Comment 3
            123
        ]"#
        .unindent_text(),
    );

    check_array_replace(
        r#"[
            {
                "key": "value"
            },
            {
                "key": "value2"
            }
        ]"#
        .unindent_text(),
        0,
        &[],
        None,
        r#"[
            {
                "key": "value2"
            }
        ]"#
        .unindent_text(),
    );

    check_array_replace(
        r#"[
            {
                "key": "value"
            },
            {
                "key": "value2"
            },
            {
                "key": "value3"
            },
        ]"#
        .unindent_text(),
        1,
        &[],
        None,
        r#"[
            {
                "key": "value"
            },
            {
                "key": "value3"
            },
        ]"#
        .unindent_text(),
    );

    check_array_replace(
        r#""#,
        2,
        &[],
        Some(json!(42)),
        r#"[
            42
        ]"#
        .unindent_text(),
    );

    check_array_replace(
        r#""#,
        2,
        &["foo", "bar"],
        Some(json!(42)),
        r#"[
            {
                "foo": {
                    "bar": 42
                }
            }
        ]"#
        .unindent_text(),
    );
}

#[test]
fn array_append() {
    #[track_caller]
    fn check_array_append(input: impl ToString, value: Value, expected: impl ToString) {
        let input = input.to_string();
        let result = append_top_level_array_value_in_json_text(&input, &value, 4);
        let mut result_str = input;
        result_str.replace_range(result.0, &result.1);
        assert_eq!(expected.to_string(), result_str);
    }
    check_array_append(r#"[1, 3, 3]"#, json!(4), r#"[1, 3, 3, 4]"#);
    check_array_append(r#"[1, 3, 3,]"#, json!(4), r#"[1, 3, 3, 4]"#);
    check_array_append(r#"[1, 3, 3   ]"#, json!(4), r#"[1, 3, 3, 4]"#);
    check_array_append(r#"[1, 3, 3,   ]"#, json!(4), r#"[1, 3, 3, 4]"#);
    check_array_append(
        r#"[
            1,
            2,
            3
        ]"#
        .unindent_text(),
        json!(4),
        r#"[
            1,
            2,
            3,
            4
        ]"#
        .unindent_text(),
    );
    check_array_append(
        r#"[
            1,
            2,
            3,
        ]"#
        .unindent_text(),
        json!(4),
        r#"[
            1,
            2,
            3,
            4
        ]"#
        .unindent_text(),
    );
    check_array_append(
        r#"[
            1,
            2,
            3,
        ]"#
        .unindent_text(),
        json!({"foo": "bar", "baz": "qux"}),
        r#"[
            1,
            2,
            3,
            {
                "foo": "bar",
                "baz": "qux"
            }
        ]"#
        .unindent_text(),
    );
    check_array_append(
        r#"[ 1, 2, 3, ]"#.unindent_text(),
        json!({"foo": "bar", "baz": "qux"}),
        r#"[ 1, 2, 3, { "foo": "bar", "baz": "qux" }]"#.unindent_text(),
    );
    check_array_append(
        r#"[]"#,
        json!({"foo": "bar"}),
        r#"[
            {
                "foo": "bar"
            }
        ]"#
        .unindent_text(),
    );

    // Test with comments between array elements
    check_array_append(
        r#"[
            1,
            // Comment between elements
            2,
            /* Block comment */ 3
        ]"#
        .unindent_text(),
        json!(4),
        r#"[
            1,
            // Comment between elements
            2,
            /* Block comment */ 3,
            4
        ]"#
        .unindent_text(),
    );

    // Test with trailing comment on last element
    check_array_append(
        r#"[
            1,
            2,
            3 // Trailing comment
        ]"#
        .unindent_text(),
        json!("new"),
        r#"[
            1,
            2,
            3 // Trailing comment
        ,
            "new"
        ]"#
        .unindent_text(),
    );

    // Test empty array with comments
    check_array_append(
        r#"[
            // Empty array with comment
        ]"#
        .unindent_text(),
        json!("first"),
        r#"[
            // Empty array with comment
            "first"
        ]"#
        .unindent_text(),
    );

    // Test with multiline block comment at end
    check_array_append(
        r#"[
            1,
            2
            /*
             * This is a
             * multiline comment
             */
        ]"#
        .unindent_text(),
        json!(3),
        r#"[
            1,
            2
            /*
             * This is a
             * multiline comment
             */
        ,
            3
        ]"#
        .unindent_text(),
    );

    // Test with deep indentation
    check_array_append(
        r#"[
            1,
                2,
                    3
        ]"#
        .unindent_text(),
        json!("deep"),
        r#"[
            1,
                2,
                    3,
                    "deep"
        ]"#
        .unindent_text(),
    );

    // Test with no spacing
    check_array_append(r#"[1,2,3]"#, json!(4), r#"[1,2,3, 4]"#);

    // Test appending complex nested structure
    check_array_append(
        r#"[
            {"a": 1},
            {"b": 2}
        ]"#
        .unindent_text(),
        json!({"c": {"nested": [1, 2, 3]}}),
        r#"[
            {"a": 1},
            {"b": 2},
            {
                "c": {
                    "nested": [
                        1,
                        2,
                        3
                    ]
                }
            }
        ]"#
        .unindent_text(),
    );

    // Test array ending with comment after bracket
    check_array_append(
        r#"[
            1,
            2,
            3
        ] // Comment after array"#
            .unindent_text(),
        json!(4),
        r#"[
            1,
            2,
            3,
            4
        ] // Comment after array"#
            .unindent_text(),
    );

    // Test with inconsistent element formatting
    check_array_append(
        r#"[1,
           2,
                3,
        ]"#
        .unindent_text(),
        json!(4),
        r#"[1,
           2,
                3,
                4
        ]"#
        .unindent_text(),
    );

    // Test appending to single-line array with trailing comma
    check_array_append(
        r#"[1, 2, 3,]"#,
        json!({"key": "value"}),
        r#"[1, 2, 3, { "key": "value" }]"#,
    );

    // Test appending null value
    check_array_append(r#"[true, false]"#, json!(null), r#"[true, false, null]"#);

    // Test appending to array with only comments
    check_array_append(
        r#"[
            // Just comments here
            // More comments
        ]"#
        .unindent_text(),
        json!(42),
        r#"[
            // Just comments here
            // More comments
            42
        ]"#
        .unindent_text(),
    );

    check_array_append(
        r#""#,
        json!(42),
        r#"[
            42
        ]"#
        .unindent_text(),
    )
}

/// Apply an append to `input` and return the new text.
fn appended(input: &str, value: Value) -> String {
    let (range, replacement) = append_top_level_array_value_in_json_text(input, &value, 2);
    let mut text = input.to_owned();
    text.replace_range(range, &replacement);
    text
}

#[test]
fn keymap_shaped_append_keeps_comments_and_trailing_commas() {
    let input = "// my keys\n[\n  {\n    \"context\": \"Table\", // rows\n    \"bindings\": { \"j\": \"table::Down\" },\n  },\n]\n";
    let text = appended(input, json!({ "bindings": { "k": "table::Up" } }));
    assert!(
        text.starts_with("// my keys\n[\n") && text.contains("// rows"),
        "{text}"
    );
    let parsed = crate::jsonc::parse_jsonc_value(&text);
    assert_eq!(parsed.as_array().map(Vec::len), Some(2), "{text}");
}

#[test]
fn keymap_shaped_append_handles_crlf_tabs_and_empty_documents() {
    let crlf = "[\r\n  { \"bindings\": {} }\r\n]\r\n";
    let text = appended(crlf, json!({ "bindings": { "a": "x" } }));
    assert_eq!(
        crate::jsonc::parse_jsonc_value(&text)
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let tabs = "[\n\t{ \"bindings\": {} }\n]\n";
    let text = appended(tabs, json!(1));
    assert_eq!(
        crate::jsonc::parse_jsonc_value(&text)
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let text = appended("", json!({ "bindings": {} }));
    assert_eq!(
        crate::jsonc::parse_jsonc_value(&text)
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let text = appended("// only a comment\n", json!(1));
    assert_eq!(
        crate::jsonc::parse_jsonc_value(&text)
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn replace_and_remove_by_index_keep_neighbouring_comments() {
    let input = "[\n  1, // one\n  2, // two\n  3 // three\n]\n";
    let (range, replacement) = replace_top_level_array_value_in_json_text(
        input,
        &[] as &[&str],
        Some(&json!(20)),
        None,
        1,
        2,
    );
    let mut text = input.to_owned();
    text.replace_range(range, &replacement);
    assert!(
        text.contains("// one") && text.contains("// two") && text.contains("20"),
        "{text}"
    );
    let (range, replacement) =
        replace_top_level_array_value_in_json_text(&text, &[] as &[&str], None, None, 2, 2);
    let mut removed = text.clone();
    removed.replace_range(range, &replacement);
    let parsed = crate::jsonc::parse_jsonc_value(&removed);
    assert_eq!(parsed, json!([1, 20]), "{removed}");
}

#[test]
fn hash_n_paths_edit_nested_array_elements() {
    let input = "{\n  \"a\": [\n    { \"b\": 1 },\n    { \"b\": 2 }\n  ]\n}\n";
    let (range, replacement) =
        replace_value_in_json_text(input, &["a", "#1", "b"], 2, Some(&json!(9)), None);
    let mut text = input.to_owned();
    text.replace_range(range, &replacement);
    assert_eq!(
        crate::jsonc::parse_jsonc_value(&text),
        json!({ "a": [{ "b": 1 }, { "b": 9 }] }),
        "{text}"
    );
}
