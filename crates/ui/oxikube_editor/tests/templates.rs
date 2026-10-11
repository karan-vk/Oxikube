//! The embedded templates (E10-S09): one minimal manifest per core kind, each read by the spanned
//! YAML model without a syntax error, validated against the OpenAPI fixtures where the fixtures
//! have the kind, with a `namespace: default` placeholder the cluster's namespace replaces. The
//! kind integration test (`bins/oxikube/tests/kind_templates.rs`) checks every one against a real
//! API server and its real schemas.

#[path = "validate/common.rs"]
mod common;

use std::collections::BTreeSet;

use common::*;
use oxikube_domain::session::NamespaceSelection;
use oxikube_editor::templates::embedded;
use oxikube_editor::validate::{DiagnosticCode, Severity};
use oxikube_editor::yaml::parse;

/// The core kinds a user does not create by hand: the API server makes them.
const NOT_CREATED_BY_HAND: [&str; 3] = ["Node", "Event", "Endpoints"];

#[test]
fn every_core_kind_has_a_template() {
    let templates = embedded();
    let have: BTreeSet<(String, String)> = templates
        .iter()
        .filter(|t| t.documents == 1)
        .filter_map(|t| t.gvk.as_ref())
        .map(|gvk| (gvk.group.to_string(), gvk.kind.to_string()))
        .collect();
    let missing: Vec<String> = oxikube_testkit::kinds::core_kinds()
        .iter()
        .filter(|kind| !NOT_CREATED_BY_HAND.contains(&&*kind.gvk.kind))
        .filter(|kind| !have.contains(&(kind.gvk.group.to_string(), kind.gvk.kind.to_string())))
        .map(|kind| kind.gvk.to_string())
        .collect();
    assert_eq!(missing, Vec::<String>::new(), "kinds without a template");
    // The kinds the story names.
    for kind in [
        "Namespace",
        "Pod",
        "Deployment",
        "StatefulSet",
        "DaemonSet",
        "Job",
        "CronJob",
        "ConfigMap",
        "Secret",
        "Service",
        "Ingress",
        "PersistentVolumeClaim",
        "ServiceAccount",
        "Role",
        "RoleBinding",
        "NetworkPolicy",
        "HorizontalPodAutoscaler",
    ] {
        assert!(
            templates.iter().any(|t| &*t.name == kind),
            "{kind} has a template"
        );
    }
}

#[test]
fn template_names_are_unique() {
    let templates = embedded();
    let names: BTreeSet<String> = templates.iter().map(|t| t.name.to_lowercase()).collect();
    assert_eq!(names.len(), templates.len());
    assert!(templates.iter().all(|t| !t.summary.is_empty()));
}

#[test]
fn every_template_reads_without_a_syntax_error_and_names_its_objects() {
    for template in embedded() {
        let parsed = parse(&template.body);
        assert_eq!(parsed.diagnostics(), [], "{}", template.name);
        let objects = parsed.docs().len();
        assert_eq!(objects, template.documents, "{}", template.name);
        assert!(template.gvk.is_some(), "{} names a kind", template.name);
        // Each document says what it is and what it is called.
        let values: Vec<serde_json::Value> =
            serde_saphyr::from_multiple(&template.body).expect("valid YAML");
        for value in &values {
            for field in ["/apiVersion", "/kind", "/metadata/name"] {
                assert!(
                    value.pointer(field).and_then(|v| v.as_str()).is_some(),
                    "{}: {field}",
                    template.name
                );
            }
        }
    }
}

#[test]
fn templates_validate_without_errors_against_the_schema_fixtures() {
    for template in embedded() {
        let (_, diagnostics) = check_buffer(&template.body);
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{}: {errors:#?}", template.name);
    }
}

#[test]
fn the_pod_template_is_clean_against_the_pod_fixture() {
    let pod = embedded().into_iter().find(|t| &*t.name == "Pod").unwrap();
    assert_eq!(check(&pod.body, &pod_schema()), []);
}

#[test]
fn the_deployment_template_only_has_fields_the_trimmed_fixture_lacks() {
    let deployment = embedded()
        .into_iter()
        .find(|t| &*t.name == "Deployment")
        .unwrap();
    let diagnostics = check(&deployment.body, &deployment_schema());
    // The fixture knows `replicas`, `strategy` and `paused` only; the rest is real Deployment.
    for d in &diagnostics {
        assert_eq!(
            d.severity,
            Severity::Warning,
            "{}",
            render(&deployment.body, &diagnostics)
        );
        assert_eq!(d.code, DiagnosticCode::UnknownField);
    }
}

#[test]
fn namespaced_templates_take_the_selected_namespace_and_cluster_scoped_ones_have_none() {
    let scopes: std::collections::BTreeMap<String, bool> = oxikube_testkit::kinds::core_kinds()
        .iter()
        .map(|kind| (kind.gvk.kind.to_string(), kind.namespaced))
        .collect();
    let selection = NamespaceSelection::single("shop");
    for template in embedded() {
        let kind = template.gvk.as_ref().map(|g| g.kind.to_string()).unwrap();
        let values: Vec<serde_json::Value> = serde_saphyr::from_multiple(&template.body).unwrap();
        let has_namespace = values[0].pointer("/metadata/namespace").is_some();
        // Where the testkit knows the kind's scope, the template agrees with it.
        if let Some(namespaced) = scopes.get(&kind).filter(|_| template.documents == 1) {
            assert_eq!(*namespaced, has_namespace, "{}", template.name);
        }
        let filled = template.prefill(&selection);
        if has_namespace {
            assert_eq!(
                filled.namespace.as_deref(),
                Some("shop"),
                "{}",
                template.name
            );
            assert!(
                !filled.text.contains("namespace: default"),
                "{}: {}",
                template.name,
                filled.text
            );
        } else if kind != "ClusterRoleBinding" {
            // A binding's subjects carry a namespace of their own, which is replaced too.
            assert_eq!(filled.namespace, None, "{}", template.name);
            assert_eq!(filled.text, *template.body, "{}", template.name);
        }
        // The result is still valid YAML.
        assert_eq!(parse(&filled.text).diagnostics(), [], "{}", template.name);
    }
}

#[test]
fn the_multi_document_template_has_two_documents_in_one_namespace() {
    let both = embedded()
        .into_iter()
        .find(|t| &*t.name == "Deployment and Service")
        .unwrap();
    assert_eq!(both.documents, 2);
    let filled = both.prefill(&NamespaceSelection::single("shop"));
    assert_eq!(filled.text.matches("namespace: shop").count(), 2);
}
