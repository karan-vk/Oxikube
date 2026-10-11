use super::*;
use crate::yaml::parse;

fn read(text: &str) -> Result<(Value, Warnings), ConvertError> {
    let parsed = parse(text);
    assert!(
        parsed.diagnostics().is_empty(),
        "{:?}",
        parsed.diagnostics()
    );
    let mut warnings = Warnings::default();
    let doc = &parsed.docs()[0];
    let value = Reader::new(&parsed, doc, &mut warnings)
        .document()?
        .expect("a document");
    Ok((value, warnings))
}

fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

#[test]
fn scalars_follow_the_core_schema() {
    let (value, warnings) = read(
        "\
i: 3\n\
f: 1.5\n\
t: true\n\
n: ~\n\
e:\n\
q: \"3\"\n\
w: yes\n\
big: 123456789012345678901234567890\n\
u: 500m\n",
    )
    .unwrap();
    assert_eq!(
        value,
        map(&[
            ("i", Value::Int("3".into())),
            ("f", Value::Float("1.5".into())),
            ("t", Value::Bool(true)),
            ("n", Value::Null),
            ("e", Value::Null),
            ("q", s("3")),
            ("w", s("yes")),
            ("big", Value::Int("123456789012345678901234567890".into())),
            ("u", s("500m")),
        ])
    );
    assert_eq!(warnings.bool_words, 1);
    assert!(!warnings.lossy());
}

#[test]
fn key_order_is_the_source_order() {
    let (value, _) = read("z: 1\na: 2\nm: 3\n").unwrap();
    let Value::Map(entries) = value else {
        panic!("a mapping")
    };
    let keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["z", "a", "m"]);
}

#[test]
fn a_leading_zero_integer_is_octal_like_the_api_server() {
    let (value, warnings) = read("defaultMode: 0644\nport: 8080\n").unwrap();
    assert_eq!(
        value,
        map(&[
            ("defaultMode", Value::Int("420".into())),
            ("port", Value::Int("8080".into()))
        ])
    );
    assert_eq!(warnings.octal, 1);
}

#[test]
fn aliases_are_expanded_and_counted() {
    let (value, warnings) = read("a: &x {k: 1}\nb: *x\nc: [*x]\n").unwrap();
    let k = map(&[("k", Value::Int("1".into()))]);
    assert_eq!(
        value,
        map(&[
            ("a", k.clone()),
            ("b", k.clone()),
            ("c", Value::Seq(vec![k]))
        ])
    );
    assert_eq!(warnings.aliases, 2);
    assert!(warnings.lossy());
}

#[test]
fn merge_keys_are_expanded_and_explicit_keys_win() {
    let (value, _) = read(
        "base: &b {x: 1, y: 2}\nother: &o {z: 3, y: 9}\nuse:\n  <<: [*b, *o]\n  y: 7\n  w: 0\n",
    )
    .unwrap();
    let Value::Map(top) = value else { panic!() };
    let (_, used) = top.iter().find(|(k, _)| k == "use").unwrap();
    assert_eq!(
        *used,
        map(&[
            ("x", Value::Int("1".into())),
            ("z", Value::Int("3".into())),
            ("y", Value::Int("7".into())),
            ("w", Value::Int("0".into())),
        ])
    );
}

#[test]
fn a_quoted_merge_key_is_an_ordinary_key() {
    let (value, _) = read("\"<<\": 1\n").unwrap();
    assert_eq!(value, map(&[("<<", Value::Int("1".into()))]));
}

#[test]
fn a_merge_of_a_scalar_is_refused() {
    let error = read("a: &x 1\nb:\n  <<: *x\n").unwrap_err();
    assert!(error.message.contains("merge key"), "{error}");
}

#[test]
fn tags_are_dropped_with_a_count_except_those_that_only_state_the_type() {
    let (value, warnings) =
        read("a: !!str 123\nb: !!binary aGk=\nc: !local x\nd: &anchor !!int 5\ne: !!map {k: v}\n")
            .unwrap();
    assert_eq!(
        value,
        map(&[
            ("a", s("123")),
            ("b", s("aGk=")),
            ("c", s("x")),
            ("d", Value::Int("5".into())),
            ("e", map(&[("k", s("v"))])),
        ])
    );
    assert_eq!(warnings.tags, 2);
}

#[test]
fn a_bang_in_a_flow_list_is_not_a_tag() {
    let (_, warnings) = read("a: [!x 1, b]\nc: [\"!y\", z]\n").unwrap();
    assert_eq!(warnings.tags, 1, "only !x is a tag");
}

#[test]
fn keys_become_strings() {
    let (value, _) = read("1: a\ntrue: b\n~: c\n2.5: d\n\"q\": e\n").unwrap();
    let Value::Map(entries) = value else { panic!() };
    let keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["1", "true", "null", "2.5", "q"]);
}

#[test]
fn complex_keys_are_refused_with_a_span() {
    let text = "? [a, b]\n: v\n";
    let parsed = parse(text);
    let mut warnings = Warnings::default();
    let error = Reader::new(&parsed, &parsed.docs()[0], &mut warnings)
        .document()
        .unwrap_err();
    assert!(error.message.contains("key"), "{error}");
    assert!(error.span.is_some());
}

#[test]
fn infinity_and_nan_have_no_json_form() {
    let error = read("a: .inf\n").unwrap_err();
    assert!(error.message.contains("no JSON spelling"), "{error}");
}

#[test]
fn an_alias_bomb_is_refused_not_expanded() {
    let mut text = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
    for i in 1..9 {
        let prev = i - 1;
        text.push_str(&format!(
            "a{i}: &a{i} [*a{prev}, *a{prev}, *a{prev}, *a{prev}, *a{prev}, *a{prev}, *a{prev}, *a{prev}, *a{prev}, *a{prev}]\n"
        ));
    }
    let error = read(&text).unwrap_err();
    assert!(error.message.contains("too many values"), "{error}");
}

#[test]
fn tags_are_found_in_the_gap_before_a_node_and_nowhere_else() {
    assert_eq!(tag_in(": &x !!str "), Some("!!str"));
    assert_eq!(tag_in(": !!str &x "), Some("!!str"));
    assert_eq!(tag_in(": &x "), None);
    assert_eq!(tag_in(": "), None);
    assert_eq!(tag_in("{!t "), Some("!t"));
    assert_eq!(tag_in(", "), None);
    assert_eq!(tag_in(": # not !a tag\n  "), None);
    assert_eq!(tag_in(": !x # but !y is a comment\n"), Some("!x"));
}

#[test]
fn a_bang_inside_a_quoted_key_is_not_a_tag() {
    let (value, warnings) = read("\"{!\": null\n\"[!x\": 1\n").unwrap();
    assert_eq!(
        value,
        map(&[("{!", Value::Null), ("[!x", Value::Int("1".into()))])
    );
    assert_eq!(warnings.tags, 0);
}

#[test]
fn a_tag_of_an_earlier_document_does_not_leak_into_the_next() {
    let parsed = parse("a: !x 1\n---\nb: 2\n");
    let mut warnings = Warnings::default();
    Reader::new(&parsed, &parsed.docs()[1], &mut warnings)
        .document()
        .unwrap();
    assert_eq!(warnings.tags, 0);
}
