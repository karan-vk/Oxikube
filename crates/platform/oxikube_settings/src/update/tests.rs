//! Comment-preserving typed edits: nested keys, arrays, comments, trailing commas, layers.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ClusterId;
use serde_json::json;

use super::new_text_for_update;
use crate::jsonc::parse_jsonc_object;
use crate::test_support::{GeneralSettings, TerminalSettings};

const USER: &str = r#"// My settings
{
  // Bigger text for demos.
  "terminal": {
    "font_size": 14, // points
    "shell": "zsh",
    "fnot": "unknown keys survive edits",
  },
  /* block comment */
  "ui_scale": 1.5,
}
"#;

#[test]
fn changes_a_nested_value_and_keeps_everything_else() {
    let text = new_text_for_update::<TerminalSettings>(USER, None, |content| {
        content.font_size = Some(18.0);
    })
    .unwrap();
    assert_eq!(
        text,
        USER.replace("\"font_size\": 14,", "\"font_size\": 18.0,")
    );
}

#[test]
fn an_unchanged_update_leaves_the_text_identical() {
    let text = new_text_for_update::<TerminalSettings>(USER, None, |_| {}).unwrap();
    assert_eq!(text, USER);
}

#[test]
fn adds_a_missing_field_inside_an_existing_section() {
    let text = new_text_for_update::<TerminalSettings>(USER, None, |content| {
        content.args = Some(vec!["-l".into()]);
    })
    .unwrap();
    assert!(text.starts_with("// My settings\n{\n  // Bigger text for demos.\n"));
    assert!(text.contains("// points") && text.contains("/* block comment */"));
    let parsed = parse_jsonc_object(&text).unwrap();
    assert_eq!(parsed["terminal"]["args"], json!(["-l"]));
    assert_eq!(
        parsed["terminal"]["fnot"],
        json!("unknown keys survive edits")
    );
    assert_eq!(parsed["ui_scale"], json!(1.5));
}

#[test]
fn replaces_arrays_as_whole_values() {
    let input = "{\n  \"terminal\": {\n    \"args\": [\"-i\"], // login\n  },\n}\n";
    let text = new_text_for_update::<TerminalSettings>(input, None, |content| {
        content.args = Some(vec!["-l".into(), "-c".into()]);
    })
    .unwrap();
    assert_eq!(
        text,
        "{\n  \"terminal\": {\n    \"args\": [\n      \"-l\",\n      \"-c\"\n    ], // login\n  },\n}\n"
    );
}

#[test]
fn clearing_a_field_removes_its_pair() {
    let text = new_text_for_update::<TerminalSettings>(USER, None, |content| {
        content.shell = None;
    })
    .unwrap();
    assert!(!text.contains("zsh"));
    assert!(text.contains("\"font_size\": 14, // points"));
    let parsed = parse_jsonc_object(&text).unwrap();
    assert!(parsed["terminal"].get("shell").is_none());
}

#[test]
fn creates_a_missing_section() {
    let input = "{\n    \"ui_scale\": 2\n}\n";
    let text = new_text_for_update::<TerminalSettings>(input, None, |content| {
        content.shell = Some("fish".into());
    })
    .unwrap();
    // Inserted before the first key, with the file's four-space indentation.
    assert_eq!(
        text,
        "{\n    \"terminal\": {\n        \"shell\": \"fish\"\n    },\n    \"ui_scale\": 2\n}\n"
    );
}

#[test]
fn edits_root_level_settings_without_touching_sections() {
    let text = new_text_for_update::<GeneralSettings>(USER, None, |content| {
        content.read_only = Some(true);
        content.ui_scale = Some(2.0);
    })
    .unwrap();
    assert!(text.contains("\"ui_scale\": 2.0,"));
    assert!(text.contains("\"font_size\": 14, // points"));
    let parsed = parse_jsonc_object(&text).unwrap();
    assert_eq!(parsed["read_only"], json!(true));
}

#[test]
fn edits_a_cluster_layer() {
    let id: ClusterId = "00000000000000aa".parse().unwrap();
    let text = new_text_for_update::<TerminalSettings>(USER, Some(&id), |content| {
        content.font_size = Some(20.0);
    })
    .unwrap();
    let parsed = parse_jsonc_object(&text).unwrap();
    assert_eq!(
        parsed["clusters"][id.as_str()]["terminal"]["font_size"],
        json!(20.0)
    );
    // The global value is untouched.
    assert_eq!(parsed["terminal"]["font_size"], json!(14));

    // A second edit lands inside the now-existing cluster object.
    let text = new_text_for_update::<GeneralSettings>(&text, Some(&id), |content| {
        content.read_only = Some(true);
    })
    .unwrap();
    let parsed = parse_jsonc_object(&text).unwrap();
    assert_eq!(parsed["clusters"][id.as_str()]["read_only"], json!(true));
    assert_eq!(
        parsed["clusters"][id.as_str()]["terminal"]["font_size"],
        json!(20.0)
    );
}

#[test]
fn an_empty_file_becomes_an_object() {
    let text = new_text_for_update::<TerminalSettings>("", None, |content| {
        content.font_size = Some(11.0);
    })
    .unwrap();
    assert_eq!(
        parse_jsonc_object(&text).unwrap()["terminal"]["font_size"],
        json!(11.0)
    );
}

#[test]
fn refuses_to_edit_a_broken_file() {
    let err =
        new_text_for_update::<TerminalSettings>("{ \"terminal\": ", None, |_| {}).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);

    let err = new_text_for_update::<TerminalSettings>(
        r#"{"terminal": {"font_size": "big"}}"#,
        None,
        |content| content.shell = Some("sh".into()),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("font_size"), "{}", err.message());
}
