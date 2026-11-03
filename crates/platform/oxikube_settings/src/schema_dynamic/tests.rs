use serde_json::json;

use super::*;

fn field() -> serde_json::Value {
    json!({
        "type": ["string", "null"],
        "x-oxikube-dynamic-enum": "themes",
        "properties": { "nested": { "x-oxikube-dynamic-enum": "fonts", "type": "string" } },
    })
}

#[test]
fn a_supplied_list_becomes_an_enum_that_keeps_null() {
    let mut schema = json!({ "properties": { "light": field() } });
    DynamicEnums::new()
        .with(THEMES, ["A".to_owned(), "B".to_owned(), "A".to_owned()])
        .apply(&mut schema);
    assert_eq!(
        schema["properties"]["light"]["enum"],
        json!(["A", "B", null])
    );
    // The nested field names a list nobody supplied: untouched.
    assert!(schema["properties"]["light"]["properties"]["nested"]["enum"].is_null());
}

#[test]
fn any_of_with_a_null_branch_keeps_null_and_plain_strings_do_not() {
    let mut schema = json!({
        "a": { "anyOf": [{ "type": "string" }, { "type": "null" }], "x-oxikube-dynamic-enum": "fonts" },
        "b": { "type": "string", "x-oxikube-dynamic-enum": "fonts" },
    });
    DynamicEnums::new()
        .with(FONTS, ["Menlo".to_owned()])
        .apply(&mut schema);
    assert_eq!(schema["a"]["enum"], json!(["Menlo", null]));
    assert_eq!(schema["b"]["enum"], json!(["Menlo"]));
}

#[test]
fn empty_lists_and_no_lists_leave_the_schema_alone() {
    let before = json!({ "a": field() });
    let mut schema = before.clone();
    DynamicEnums::new().apply(&mut schema);
    DynamicEnums::new().with(THEMES, []).apply(&mut schema);
    assert_eq!(schema, before);
}
