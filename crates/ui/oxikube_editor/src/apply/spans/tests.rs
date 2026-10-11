use oxikube_domain::FieldCause;

use super::*;

const DEPLOYMENT: &str = "\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  labels:
    app.kubernetes.io/name: web
spec:
  replicas: two
  template:
    spec:
      containers:
        - name: sidecar
          image: busybox
        - name: web
          image: \"\"
          ports:
            - containerPort: 80
              protocol: TCP
";

fn cause(field: &str, message: &str) -> FieldCause {
    FieldCause {
        field: field.into(),
        message: message.into(),
        reason: "FieldValueInvalid".into(),
        manager: None,
    }
}

fn marked(field: &str) -> (String, String) {
    let placed = place_causes(DEPLOYMENT, &[cause(field, "Invalid value")]);
    assert!(placed.unplaced.is_empty(), "{field}: {:?}", placed.unplaced);
    let diagnostic = &placed.diagnostics[0];
    assert_eq!(diagnostic.level, DiagnosticLevel::Error);
    assert_eq!(diagnostic.code.as_deref(), Some(SERVER_CODE));
    (
        DEPLOYMENT[diagnostic.range.clone()].to_owned(),
        diagnostic.message.clone(),
    )
}

fn line_of(range_start: usize) -> usize {
    DEPLOYMENT[..range_start].matches('\n').count()
}

#[test]
fn a_validation_field_marks_its_key() {
    let (text, message) = marked("spec.replicas");
    assert_eq!(text, "replicas");
    assert_eq!(message, "spec.replicas: Invalid value");
}

#[test]
fn indexes_and_keys_with_dots_resolve() {
    let placed = place_causes(
        DEPLOYMENT,
        &[cause(
            "spec.template.spec.containers[1].image",
            "Required value",
        )],
    );
    let range = placed.diagnostics[0].range.clone();
    assert_eq!(&DEPLOYMENT[range.clone()], "image");
    assert_eq!(line_of(range.start), 14, "the web container's image");

    let (text, _) = marked("metadata.labels[app.kubernetes.io/name]");
    assert_eq!(text, "app.kubernetes.io/name");
}

#[test]
fn server_side_apply_item_selectors_resolve() {
    let placed = place_causes(
        DEPLOYMENT,
        &[cause(
            ".spec.template.spec.containers[name=\"web\"].ports[containerPort=80,protocol=\"TCP\"]",
            "conflict",
        )],
    );
    let range = placed.diagnostics[0].range.clone();
    assert_eq!(&DEPLOYMENT[range.clone()], "containerPort");
    assert_eq!(line_of(range.start), 16);
}

#[test]
fn a_missing_field_goes_on_its_nearest_present_parent() {
    let (text, message) = marked("spec.template.spec.containers[0].resources.limits");
    assert_eq!(text, "name", "the first key of the sidecar item");
    assert!(message.ends_with("(not in the manifest)"), "{message}");
    let (text, _) = marked("spec.selector");
    assert_eq!(text, "spec");
}

#[test]
fn a_cause_without_a_place_is_returned_as_text() {
    let placed = place_causes(
        DEPLOYMENT,
        &[
            cause("", "the server could not find the requested resource"),
            cause("status", "Forbidden"),
        ],
    );
    assert!(placed.diagnostics.is_empty());
    assert_eq!(
        placed.unplaced,
        [
            "the server could not find the requested resource",
            "status: Forbidden"
        ]
    );
}

#[test]
fn field_paths_parse() {
    use Step::*;
    assert_eq!(
        parse_field(".spec.containers[name=\"a,b\"].ports[0]").unwrap(),
        [
            Key("spec".into()),
            Key("containers".into()),
            Item(vec![("name".into(), "a,b".into())]),
            Key("ports".into()),
            Index(0),
        ]
    );
    assert_eq!(
        parse_field("metadata.annotations[a.b/c]").unwrap(),
        [
            Key("metadata".into()),
            Key("annotations".into()),
            Key("a.b/c".into())
        ]
    );
    assert_eq!(
        parse_field("a[\"x]y\"]").unwrap(),
        [Key("a".into()), Key("x]y".into())]
    );
    assert_eq!(parse_field("a[unclosed"), None);
    assert_eq!(parse_field("a..b"), None);
}
