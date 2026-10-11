//! E10-S10 properties: for generated documents, YAML to JSON to YAML keeps the data (compared as
//! parsed values and as text), both writers produce text their own readers accept, strings that
//! look like numbers, booleans or null stay strings, and the dropped-comment count is the number
//! of comments in the source.

use oxikube_editor::json::{
    Value, count_comments, first_error, json_to_yaml, to_json, to_yaml, yaml_to_json,
};
use oxikube_editor::yaml::parse;
use proptest::prelude::*;

/// Strings that are hard to write: look like other types, hold structure characters, break
/// lines, use unusual Unicode.
fn tricky() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("yes".to_owned()),
        Just("No".to_owned()),
        Just("on".to_owned()),
        Just("y".to_owned()),
        Just("123".to_owned()),
        Just("0644".to_owned()),
        Just("1e3".to_owned()),
        Just("1.5".to_owned()),
        Just(".inf".to_owned()),
        Just("true".to_owned()),
        Just("null".to_owned()),
        Just("~".to_owned()),
        Just(String::new()),
        Just(" lead".to_owned()),
        Just("trail ".to_owned()),
        Just("a: b".to_owned()),
        Just("a #b".to_owned()),
        Just("- x".to_owned()),
        Just("---".to_owned()),
        Just("line\nbreak".to_owned()),
        Just("one\ntwo\n".to_owned()),
        Just("\n\n".to_owned()),
        Just("tab\there".to_owned()),
        Just("quote\"and\\slash".to_owned()),
        Just("\u{85}\u{2028}\u{feff}\u{fffe}".to_owned()),
        Just("日本語 😀 é".to_owned()),
        Just("<<".to_owned()),
        Just("2001-12-14".to_owned()),
        Just("12:30".to_owned()),
    ]
}

fn string() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => tricky(),
        3 => "[ -~]{0,16}",
        2 => any::<String>().prop_map(|s| s.chars().take(24).collect()),
    ]
}

fn leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| Value::Int(n.to_string())),
        "[1-9][0-9]{19,40}".prop_map(Value::Int),
        (any::<i32>(), 0..1000u32).prop_map(|(a, b)| Value::Float(format!("{a}.{b:03}"))),
        string().prop_map(Value::Str),
    ]
}

fn value() -> impl Strategy<Value = Value> {
    leaf().prop_recursive(4, 40, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Seq),
            prop::collection::vec((string(), inner), 0..4).prop_map(|entries| {
                let mut seen = std::collections::HashSet::new();
                Value::Map(
                    entries
                        .into_iter()
                        .filter(|(key, _)| seen.insert(key.clone()))
                        .collect(),
                )
            }),
        ]
    })
}

/// Reads `yaml` back into a `Value` through the same path the conversion uses (YAML to JSON to
/// the `Value`'s JSON text), so equality is on the data.
fn data_of_yaml(yaml: &str) -> String {
    yaml_to_json(yaml)
        .unwrap_or_else(|e| panic!("{e}\n---\n{yaml}"))
        .text
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn yaml_written_by_us_reads_back_to_the_same_data(v in value()) {
        let yaml = to_yaml(&v);
        prop_assert!(parse(&yaml).diagnostics().is_empty(), "{:?}\n{yaml}", parse(&yaml).diagnostics());
        prop_assert_eq!(data_of_yaml(&yaml), to_json(&v), "yaml:\n{}", yaml);
    }

    #[test]
    fn json_written_by_us_is_strict_json_and_reads_back_to_the_same_data(v in value()) {
        let json = to_json(&v);
        prop_assert_eq!(first_error(&json), None, "{}", json);
        let yaml = json_to_yaml(&json).unwrap_or_else(|e| panic!("{e}\n{json}")).text;
        prop_assert_eq!(yaml, to_yaml(&v), "json:\n{}", json);
    }

    #[test]
    fn yaml_to_json_to_yaml_is_stable(v in value()) {
        let yaml = to_yaml(&v);
        let json = yaml_to_json(&yaml).unwrap();
        let back = json_to_yaml(&json.text).unwrap();
        prop_assert_eq!(&back.text, &yaml);
        prop_assert_eq!(json.warnings.comments, 0);
        prop_assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    }

    #[test]
    fn the_warning_counts_exactly_the_comments_in_the_source(
        trailing in prop::collection::vec(prop::option::of("[ -~]{0,12}"), 8),
        between in prop::collection::vec(prop::option::of("[ -~]{0,12}"), 8),
    ) {
        let lines = [
            "apiVersion: v1",
            "kind: ConfigMap",
            "metadata:",
            "  name: cm",
            "data:",
            "  a: \"quoted # not a comment\"",
            "  b: [x, y]",
            "  c: 'single'",
        ];
        let mut yaml = String::new();
        let mut comments = 0;
        for (i, line) in lines.iter().enumerate() {
            if let Some(text) = &between[i] {
                yaml.push_str(&format!("# {text}\n"));
                comments += 1;
            }
            yaml.push_str(line);
            if let Some(text) = &trailing[i] {
                yaml.push_str(&format!(" # {text}"));
                comments += 1;
            }
            yaml.push('\n');
        }
        prop_assert_eq!(count_comments(&parse(&yaml)), comments, "{}", yaml);
        let converted = yaml_to_json(&yaml).unwrap();
        prop_assert_eq!(converted.warnings.comments, comments);
        // The comments change nothing else.
        let plain: String = lines.iter().map(|l| format!("{l}\n")).collect();
        prop_assert_eq!(converted.text, yaml_to_json(&plain).unwrap().text);
        prop_assert_eq!(converted.warnings.summary().is_some(), comments > 0);
    }
}

/// The value as `serde_json` sees it, `None` when it cannot be (an integer past `i64`).
fn to_serde(v: &Value) -> Option<serde_json::Value> {
    use serde_json::Value as J;
    Some(match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(text) => J::from(text.parse::<i64>().ok()?),
        Value::Float(text) => J::from(text.parse::<f64>().ok()?),
        Value::Str(s) => J::String(s.clone()),
        Value::Seq(items) => J::Array(items.iter().map(to_serde).collect::<Option<_>>()?),
        Value::Map(entries) => J::Object(
            entries
                .iter()
                .map(|(k, v)| Some((k.clone(), to_serde(v)?)))
                .collect::<Option<_>>()?,
        ),
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// An independent YAML reader (serde-saphyr) agrees with the writer about every string,
    /// number, boolean and null, so the quoting rules are not just self-consistent.
    #[test]
    fn an_independent_yaml_reader_reads_the_written_yaml_as_the_same_data(v in value()) {
        let Some(expected) = to_serde(&v) else { return Ok(()); };
        let yaml = to_yaml(&v);
        let read: serde_json::Value = serde_saphyr::from_str(&yaml)
            .unwrap_or_else(|e| panic!("{e}\n---\n{yaml}"));
        prop_assert_eq!(read, expected, "yaml:\n{}", yaml);
    }

    /// The same for the JSON writer and `serde_json`.
    #[test]
    fn serde_json_reads_the_written_json_as_the_same_data(v in value()) {
        let Some(expected) = to_serde(&v) else { return Ok(()); };
        let json = to_json(&v);
        let read: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("{e}\n---\n{json}"));
        prop_assert_eq!(read, expected, "json:\n{}", json);
    }
}
