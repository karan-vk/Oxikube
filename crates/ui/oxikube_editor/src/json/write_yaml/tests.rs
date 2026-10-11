use super::*;

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
    )
}

#[test]
fn writes_the_kubectl_layout() {
    let value = map(vec![
        ("apiVersion", s("apps/v1")),
        ("kind", s("Deployment")),
        (
            "spec",
            map(vec![
                ("replicas", Value::Int("2".into())),
                (
                    "containers",
                    Value::Seq(vec![
                        map(vec![
                            ("name", s("web")),
                            ("args", Value::Seq(vec![s("--port=80"), s("-v")])),
                        ]),
                        map(vec![("name", s("side"))]),
                    ]),
                ),
                ("empty", map(vec![])),
                ("none", Value::Seq(vec![])),
                ("nil", Value::Null),
            ]),
        ),
    ]);
    assert_eq!(
        to_yaml(&value),
        "\
apiVersion: apps/v1
kind: Deployment
spec:
  replicas: 2
  containers:
  - name: web
    args:
    - --port=80
    - -v
  - name: side
  empty: {}
  none: []
  nil: null
"
    );
}

#[test]
fn strings_that_look_like_other_types_stay_quoted() {
    let value = map(vec![
        ("a", s("yes")),
        ("b", s("on")),
        ("c", s("123")),
        ("d", s("true")),
        ("e", s("null")),
        ("f", s("~")),
        ("g", s("")),
        ("h", s("1.5")),
        ("i", s("0644")),
        ("j", s("2001-12-14")),
        ("k", s("500m")),
    ]);
    assert_eq!(
        to_yaml(&value),
        "\
a: \"yes\"
b: \"on\"
c: \"123\"
d: \"true\"
e: \"null\"
f: \"~\"
g: \"\"
h: \"1.5\"
i: \"0644\"
j: \"2001-12-14\"
k: 500m
"
    );
}

#[test]
fn typed_scalars_are_written_as_themselves() {
    let value = map(vec![
        ("i", Value::Int("123456789012345678901234567890".into())),
        ("f", Value::Float("1.5e3".into())),
        ("t", Value::Bool(true)),
        ("nothing", Value::Null),
    ]);
    assert_eq!(
        to_yaml(&value),
        "i: 123456789012345678901234567890\nf: 1.5e3\nt: true\nnothing: null\n"
    );
}

#[test]
fn multi_line_text_is_a_literal_block_when_that_is_exact() {
    let value = map(vec![
        ("one", s("a\nb\n")),
        ("strip", s("a\nb")),
        ("keep", s("a\nb\n\n")),
        ("blank", s("a\n\nb\n")),
        ("lead", s("\nx\n")),
        ("list", Value::Seq(vec![s("l1\nl2\n"), s("k")])),
    ]);
    assert_eq!(
        to_yaml(&value),
        "\
one: |
  a
  b
strip: |-
  a
  b
keep: |+
  a
  b

blank: |
  a

  b
lead: |

  x
list:
- |
  l1
  l2
- k
"
    );
}

#[test]
fn multi_line_text_falls_back_to_a_quoted_string_when_a_block_would_not_read_back() {
    let value = map(vec![
        ("indented", s(" x\ny\n")),
        ("only_newlines", s("\n\n")),
        ("control", s("a\u{1}\nb")),
        ("cr", s("a\r\nb")),
    ]);
    assert_eq!(
        to_yaml(&value),
        "\
indented: \" x\\ny\\n\"
only_newlines: \"\\n\\n\"
control: \"a\\x01\\nb\"
cr: \"a\\r\\nb\"
"
    );
}

#[test]
fn keys_are_quoted_when_needed() {
    let value = map(vec![
        ("plain", Value::Null),
        ("with space", Value::Null),
        ("a: b", Value::Null),
        ("123", Value::Null),
        ("", Value::Null),
        (".dockerconfigjson", Value::Null),
    ]);
    assert_eq!(
        to_yaml(&value),
        "plain: null\nwith space: null\n\"a: b\": null\n\"123\": null\n\"\": null\n.dockerconfigjson: null\n"
    );
}

#[test]
fn nested_sequences_and_scalar_roots() {
    let value = Value::Seq(vec![
        Value::Seq(vec![s("a"), s("b")]),
        Value::Seq(vec![]),
        map(vec![("k", Value::Seq(vec![s("x")]))]),
    ]);
    assert_eq!(to_yaml(&value), "- - a\n  - b\n- []\n- k:\n  - x\n");
    assert_eq!(to_yaml(&s("hello")), "hello\n");
    assert_eq!(to_yaml(&s("yes")), "\"yes\"\n");
    assert_eq!(to_yaml(&Value::Null), "null\n");
    assert_eq!(to_yaml(&map(vec![])), "{}\n");
}
