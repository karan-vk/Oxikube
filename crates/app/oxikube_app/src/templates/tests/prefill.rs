//! The namespace placeholder.

use oxikube_domain::session::NamespaceSelection;

use super::*;

fn configmap() -> Template {
    Template::embedded("ConfigMap", "", CONFIGMAP)
}

#[test]
fn one_selected_namespace_goes_into_the_placeholder_without_a_note() {
    let filled = configmap().prefill(&NamespaceSelection::single("shop"));
    assert_eq!(filled.namespace.as_deref(), Some("shop"));
    assert_eq!(filled.note, None);
    assert_eq!(
        filled.text,
        CONFIGMAP.replace("namespace: default", "namespace: shop")
    );
}

#[test]
fn all_namespaces_default_to_default_and_say_so() {
    let filled = configmap().prefill(&NamespaceSelection::All);
    assert_eq!(filled.text, CONFIGMAP, "default stays default");
    assert_eq!(filled.namespace.as_deref(), Some("default"));
    assert_eq!(
        filled.note.as_deref(),
        Some("All namespaces are selected: the namespace is set to default")
    );
}

#[test]
fn several_selected_namespaces_take_the_first_and_say_so() {
    let filled = configmap().prefill(&NamespaceSelection::from_names(["shop", "api"]));
    assert_eq!(filled.namespace.as_deref(), Some("api"));
    assert!(filled.text.contains("namespace: api"));
    assert_eq!(
        filled.note.as_deref(),
        Some("2 namespaces are selected: the namespace is set to api, the first")
    );
}

#[test]
fn a_cluster_scoped_template_has_no_namespace_and_no_note() {
    let template = &embedded()[0];
    let filled = template.prefill(&NamespaceSelection::All);
    assert_eq!(filled.namespace, None);
    assert_eq!(filled.note, None);
    assert_eq!(filled.text, *template.body);
}

#[test]
fn only_the_placeholder_is_replaced() {
    let body = "\
kind: RoleBinding
metadata:
  namespace: default   # replaced
subjects:
  - kind: ServiceAccount
    name: default
    namespace: \"default\"
  - kind: ServiceAccount
    name: other
    namespace: kube-system
env:
  - name: namespaceSelector
    value: default
";
    let filled = Template::embedded("RB", "", body).prefill(&NamespaceSelection::single("shop"));
    assert_eq!(
        filled.text,
        "\
kind: RoleBinding
metadata:
  namespace: shop   # replaced
subjects:
  - kind: ServiceAccount
    name: default
    namespace: shop
  - kind: ServiceAccount
    name: other
    namespace: kube-system
env:
  - name: namespaceSelector
    value: default
"
    );
}

#[test]
fn line_endings_and_a_missing_final_newline_are_kept() {
    let body = "metadata:\r\n  namespace: default\r\n  name: a\r\nx:\r\n  namespace: default";
    let filled = Template::embedded("T", "", body).prefill(&NamespaceSelection::single("b"));
    assert_eq!(
        filled.text,
        "metadata:\r\n  namespace: b\r\n  name: a\r\nx:\r\n  namespace: b"
    );
}
