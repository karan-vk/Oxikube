use super::*;

const DEPLOYMENT: &str = "\
# The web tier.
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  labels:
    app: web   # selector label
spec:
  replicas: 3
  paused: false
  selector:
    matchLabels:
      app: web
  template:
    spec:
      containers:
        - name: web
          image: nginx:1.27
          args: [\"--port=80\", \"-v\"]
          env:
            - name: MODE
              value: \"yes\"
            - name: COUNT
              value: \"123\"
          resources:
            limits: {cpu: 500m, memory: 1Gi}
";

fn json(text: &str) -> Converted {
    yaml_to_json(text).unwrap_or_else(|e| panic!("{e}"))
}

fn yaml(text: &str) -> Converted {
    json_to_yaml(text).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_deployment_converts_in_key_order_and_counts_its_comments() {
    let converted = json(DEPLOYMENT);
    assert_eq!(converted.to, Language::Json);
    assert_eq!(converted.warnings.comments, 2);
    assert!(converted.warnings.lossy());
    assert_eq!(
        converted.warnings.summary().as_deref(),
        Some("2 comments dropped")
    );
    let text = &converted.text;
    assert!(text.starts_with("{\n  \"apiVersion\": \"apps/v1\",\n  \"kind\": \"Deployment\","));
    assert!(text.contains("\"replicas\": 3,"));
    assert!(text.contains("\"paused\": false,"));
    assert!(text.contains("\"value\": \"yes\""));
    assert!(text.contains("\"value\": \"123\""));
    assert!(text.contains("\"cpu\": \"500m\""));
    assert!(text.ends_with("}\n"));
    assert_eq!(first_error(text), None, "valid JSON");
}

#[test]
fn json_to_yaml_to_json_is_the_identity_on_the_text() {
    let first = json(DEPLOYMENT).text;
    let back = yaml(&first);
    assert!(back.warnings.is_empty());
    let again = json(&back.text);
    assert_eq!(again.text, first);
    assert_eq!(again.warnings.comments, 0);
}

#[test]
fn strings_that_look_like_other_types_survive_a_round_trip_as_strings() {
    let source = "{\"a\": \"123\", \"b\": \"true\", \"c\": \"null\", \"d\": \"~\", \"e\": \"yes\", \"f\": \"on\", \"g\": \"1.5\", \"h\": \"\", \"i\": \"0644\", \"nothing\": null, \"t\": true, \"i2\": 123}";
    let converted = yaml(source);
    let text = &converted.text;
    for line in [
        "a: \"123\"",
        "b: \"true\"",
        "c: \"null\"",
        "d: \"~\"",
        "e: \"yes\"",
        "f: \"on\"",
        "g: \"1.5\"",
        "h: \"\"",
        "i: \"0644\"",
        "nothing: null",
        "t: true",
        "i2: 123",
    ] {
        assert!(text.contains(line), "{line} in\n{text}");
    }
    assert_eq!(json(text).text, json(&json(text).text).text, "stable");
    // Back to JSON: the same data, string for string.
    let back = json(text).text;
    assert!(back.contains("\"a\": \"123\""));
    assert!(back.contains("\"e\": \"yes\""));
    assert!(back.contains("\"nothing\": null"));
    assert!(back.contains("\"i2\": 123"));
    assert_eq!(
        json(text).warnings.bool_words,
        0,
        "quoted words are no caveat"
    );
}

#[test]
fn big_integers_and_float_digits_are_kept() {
    let source = "{\"big\": 123456789012345678901234567890, \"neg\": -9007199254740993, \"f\": 0.1000000000000000055511151231257827, \"e\": 1.5e300}";
    let yaml_text = yaml(source).text;
    assert!(yaml_text.contains("big: 123456789012345678901234567890"));
    assert!(yaml_text.contains("neg: -9007199254740993"));
    assert!(yaml_text.contains("f: 0.1000000000000000055511151231257827"));
    let back = json(&yaml_text).text;
    assert!(back.contains("123456789012345678901234567890"));
    assert!(back.contains("-9007199254740993"));
    assert!(back.contains("0.1000000000000000055511151231257827"));
    assert!(back.contains("1.5e300"));
}

#[test]
fn unicode_escapes_are_decoded_in_yaml_and_written_raw_in_json() {
    let converted = yaml("{\"a\": \"\\u00e9\\u003c\\ud83d\\ude00\"}");
    assert_eq!(converted.text, "a: \"é<😀\"\n".replace("\"é<😀\"", "é<😀"));
    let back = json(&converted.text);
    assert!(back.text.contains("\"a\": \"é<😀\""), "{}", back.text);
}

#[test]
fn multi_line_strings_round_trip_through_literal_blocks() {
    let source =
        "{\"data\": {\"conf\": \"line1\\nline2\\n\", \"nonl\": \"a\\nb\", \"sp\": \" lead\\nx\"}}";
    let yaml_text = yaml(source).text;
    assert!(
        yaml_text.contains("conf: |\n    line1\n    line2\n"),
        "{yaml_text}"
    );
    let back = json(&yaml_text).text;
    assert!(back.contains("\"conf\": \"line1\\nline2\\n\""));
    assert!(back.contains("\"nonl\": \"a\\nb\""));
    assert!(back.contains("\"sp\": \" lead\\nx\""));
}

#[test]
fn several_documents_become_one_list() {
    let converted = json("a: 1\n---\nb: 2\n---\n# nothing here\n");
    assert_eq!(converted.warnings.documents, 2);
    assert_eq!(converted.warnings.comments, 1);
    let text = &converted.text;
    assert!(
        text.starts_with("{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"List\",\n  \"items\": [")
    );
    assert!(text.contains("\"a\": 1") && text.contains("\"b\": 2"));
    // One document is not wrapped.
    assert!(!json("a: 1\n").text.contains("List"));
}

#[test]
fn a_json_list_stays_a_list_going_back() {
    let list = json("kind: Pod\n---\nkind: Service\n").text;
    let back = yaml(&list);
    assert!(
        back.text
            .starts_with("apiVersion: v1\nkind: List\nitems:\n- kind: Pod\n- kind: Service\n")
    );
}

#[test]
fn aliases_merge_keys_and_tags_are_read_like_the_api_server_does() {
    let source = "\
base: &b {x: 1}
use:
  <<: *b
  y: !!str 2
mode: 0644
";
    let converted = json(source);
    assert!(converted.text.contains("\"x\": 1"));
    assert!(converted.text.contains("\"y\": \"2\""));
    assert!(converted.text.contains("\"mode\": 420"));
    assert_eq!(converted.warnings.aliases, 1);
    assert_eq!(converted.warnings.octal, 1);
    assert!(converted.warnings.lossy(), "aliases lose their sharing");
}

#[test]
fn broken_input_is_refused_with_a_span_and_a_line() {
    let error = yaml_to_json("a: [1,\nb: 2\n").unwrap_err();
    assert!(error.message.starts_with("line "), "{error}");
    assert!(error.message.contains("fix the YAML first"), "{error}");
    assert!(error.span.is_some());

    let text = "{\"a\": 1,}";
    let error = json_to_yaml(text).unwrap_err();
    assert!(error.message.contains("trailing comma"), "{error}");
    assert_eq!(&text[error.span.unwrap()], ",");
}

#[test]
fn duplicate_keys_are_refused_not_silently_merged() {
    let error = yaml_to_json("a: 1\na: 2\n").unwrap_err();
    assert!(error.message.contains("duplicate mapping key"), "{error}");
    let error = json_to_yaml("{\"a\": 1, \"a\": 2}").unwrap_err();
    assert!(error.message.contains("duplicate mapping key"), "{error}");
}

#[test]
fn empty_input_has_nothing_to_convert() {
    assert!(yaml_to_json("").is_err());
    assert!(yaml_to_json("# only a comment\n").is_err());
    assert!(json_to_yaml("  \n").is_err());
}

#[test]
fn infinity_cannot_become_json() {
    let error = yaml_to_json("a: .inf\n").unwrap_err();
    assert!(error.message.contains("no JSON spelling"), "{error}");
}

#[test]
fn convert_picks_the_direction_from_the_target() {
    assert_eq!(
        convert("a: 1\n", Language::Json).unwrap().to,
        Language::Json
    );
    assert_eq!(
        convert("{\"a\": 1}", Language::Yaml).unwrap().text,
        "a: 1\n"
    );
}

#[test]
fn a_secret_round_trips_without_touching_its_values() {
    let secret = "\
apiVersion: v1
kind: Secret
metadata:
  name: creds
type: Opaque
data:
  password: cGFzczoxMjM0
  token: MDEyMzQ1
stringData:
  note: \"yes\"
";
    let as_json = json(secret);
    assert_eq!(as_json.warnings.comments, 0);
    let back = yaml(&as_json.text);
    assert_eq!(back.text, secret);
}

#[test]
fn absurd_nesting_is_refused_not_overflowed() {
    let text = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
    let error = yaml_to_json(&text).unwrap_err();
    assert!(error.message.contains("fix the YAML first"), "{error}");
    let error = json_to_yaml(&text).unwrap_err();
    assert!(error.message.contains("deeply"), "{error}");
}
