//! The bundled keymaps conform to the generated `keymap.schema.json`: only the properties the
//! schema declares, and bound actions that are in its `ActionName` enum.

use std::collections::BTreeSet;

use oxikube_assets::{KeymapPlatform, default_keymap, keymap_schema, vim_keymap};
use serde_json::Value;

fn parse(text: &str) -> Value {
    serde_json_lenient::from_str(text).expect("a bundled keymap is valid JSONC")
}

fn actions_in(binding: &Value, out: &mut Vec<String>) {
    match binding {
        Value::String(name) => out.push(name.clone()),
        Value::Array(parts) => {
            if let Some(Value::String(name)) = parts.first() {
                out.push(name.clone());
            }
        }
        _ => {}
    }
}

fn check(label: &str, text: &str, schema: &Value) {
    let allowed: BTreeSet<&str> = schema["$defs"]["Section"]["properties"]
        .as_object()
        .expect("Section properties")
        .keys()
        .map(String::as_str)
        .collect();
    let actions: BTreeSet<&str> = schema["$defs"]["ActionName"]["enum"]
        .as_array()
        .expect("ActionName enum")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let sections = parse(text);
    for (index, section) in sections.as_array().expect("an array").iter().enumerate() {
        let object = section.as_object().expect("a section object");
        for key in object.keys() {
            assert!(allowed.contains(key.as_str()), "{label} #{index}: `{key}`");
        }
        let mut names = Vec::new();
        for binding in object["bindings"].as_object().expect("bindings").values() {
            actions_in(binding, &mut names);
        }
        for name in names {
            assert!(actions.contains(name.as_str()), "{label} #{index}: {name}");
        }
    }
}

#[test]
fn bundled_default_and_vim_keymaps_conform_to_the_generated_schema() {
    let schema = parse(keymap_schema());
    for platform in [
        KeymapPlatform::MacOs,
        KeymapPlatform::Linux,
        KeymapPlatform::Windows,
    ] {
        check(&format!("{platform:?}"), default_keymap(platform), &schema);
    }
    check("vim", vim_keymap(), &schema);
}
