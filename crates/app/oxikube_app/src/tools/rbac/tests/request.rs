use oxikube_domain::access::Impersonate;
use oxikube_domain::ids::{Gvk, ResourceRef};
use serde_json::json;

use super::{Harness, cluster};
use crate::search::aliases::AliasTable;
use crate::tools::rbac::{CanIRequest, CanISubject};

fn request(verb: &str, resource: &str) -> CanIRequest {
    CanIRequest {
        verb: verb.into(),
        resource: resource.into(),
        ..CanIRequest::default()
    }
}

#[test]
fn resources_resolve_through_the_alias_table() {
    let table = AliasTable::new();
    let check = request("get", "po").to_check(&table).expect("check");
    assert_eq!(&*check.gvr.resource, "pods");
    assert_eq!(&*check.gvr.group, "");
    let check = request("get", "deployments.apps")
        .to_check(&table)
        .expect("check");
    assert_eq!(&*check.gvr.resource, "deployments");
    assert_eq!(&*check.gvr.group, "apps");
}

#[test]
fn subresources_and_unknown_resources_are_kept_as_typed() {
    let table = AliasTable::new();
    let check = request("create", "pods/exec")
        .to_check(&table)
        .expect("check");
    assert_eq!(check.subresource.as_deref(), Some("exec"));
    let check = request("get", "widgets.acme.io")
        .to_check(&table)
        .expect("check");
    assert_eq!(
        (&*check.gvr.resource, &*check.gvr.group),
        ("widgets", "acme.io")
    );
}

#[test]
fn the_subject_becomes_an_impersonation_and_blank_fields_are_dropped() {
    let mut req = request("DELETE", "pods");
    req.namespace = Some(String::new());
    req.subject = CanISubject::ServiceAccount {
        namespace: "shop".into(),
        name: "bot".into(),
    };
    let check = req.to_check(&AliasTable::new()).expect("check");
    assert_eq!(check.verb, "delete");
    assert_eq!(check.namespace, None);
    assert_eq!(
        check.impersonate,
        Some(Impersonate::service_account("shop", "bot"))
    );
}

#[test]
fn empty_verb_or_resource_is_a_validation_error() {
    let table = AliasTable::new();
    assert!(request("", "pods").to_check(&table).is_err());
    assert!(request("get", "/exec").to_check(&table).is_err());
}

#[test]
fn subjects_parse_from_kind_words() {
    assert_eq!(
        CanISubject::parse("sa", "shop/bot", None).expect("sa"),
        CanISubject::ServiceAccount {
            namespace: "shop".into(),
            name: "bot".into()
        }
    );
    assert!(CanISubject::parse("serviceaccount", "bot", None).is_err());
    assert!(CanISubject::parse("robot", "x", None).is_err());
    assert!(CanISubject::parse("user", " ", None).is_err());
}

#[test]
fn args_without_a_verb_are_rejected() {
    assert!(CanIRequest::from_args(&json!({"resource": "pods"})).is_err());
    let req = CanIRequest::from_args(
        &json!({"verb": "get", "resource": "pods", "subject": {"kind": "group", "name": "devs"}}),
    )
    .expect("request");
    assert_eq!(req.subject, CanISubject::Group("devs".into()));
}

#[tokio::test]
async fn prefill_spells_the_selected_object_as_kubectl_does() {
    let h = Harness::new();
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0");
    let req = h.service.prefill(&cluster(), &pod, None);
    assert_eq!(
        (
            req.verb.as_str(),
            req.resource.as_str(),
            req.namespace.as_deref(),
            req.name.as_deref()
        ),
        ("get", "pods", Some("shop"), Some("web-0"))
    );
    let deploy = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    );
    let req = h.service.prefill(&cluster(), &deploy, Some("delete"));
    assert_eq!(
        (req.verb.as_str(), req.resource.as_str()),
        ("delete", "deployments.apps")
    );
}
