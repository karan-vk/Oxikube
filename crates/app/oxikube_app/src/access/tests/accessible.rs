//! The accessible-namespaces computation.

use super::*;
use crate::access::{NamespaceOrigin, accessible_namespaces};
use oxikube_domain::OxiError;
use oxikube_domain::access::AccessRule;
use oxikube_domain::session::NamespaceSelection;

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
async fn the_setting_is_an_override_and_nothing_is_reviewed() {
    let (service, port, _clock) = service(FakeAccessReviewPort::new());

    let found =
        accessible_namespaces(&service, &names(&[" web ", "api", ""]), names(&["dev"])).await;

    assert_eq!(found.names, ["api", "web"]);
    assert_eq!(found.origin, NamespaceOrigin::Override);
    assert!(port.recorded_calls().is_empty());
    assert_eq!(
        found.selection(),
        NamespaceSelection::from_names(["api", "web"])
    );
}

#[tokio::test]
async fn candidates_are_kept_when_the_user_may_read_something_there() {
    let (service, _port, _clock) = service(
        FakeAccessReviewPort::new()
            .with_namespace_rules("dev", reads(&["pods"]))
            .with_namespace_rules("prod", AccessRules::none()),
    );

    let found = accessible_namespaces(&service, &[], names(&["prod", "dev", "dev"])).await;

    assert_eq!(found.names, ["dev"]);
    assert_eq!(found.origin, NamespaceOrigin::Reviewed);
}

#[tokio::test]
async fn cluster_scoped_grants_do_not_make_a_namespace_accessible() {
    let nodes_only =
        AccessRules::none().with_rule(AccessRule::granting(&["list"], &[""], &["nodes"], &[]));
    let (service, _port, _clock) = service(FakeAccessReviewPort::new().with_rules(nodes_only));

    let found = accessible_namespaces(&service, &[], names(&["dev"])).await;

    assert!(found.names.is_empty());
    assert_eq!(found.selection(), NamespaceSelection::All);
}

#[tokio::test]
async fn wildcards_and_a_failed_review_keep_the_candidate() {
    let admin = AccessRules::all_access();
    let (service, port, _clock) =
        service(FakeAccessReviewPort::new().with_namespace_rules("a", admin));
    port.script().rules.push_err(OxiError::network("down"));

    let found = accessible_namespaces(&service, &[], names(&["a", "b"])).await;

    assert_eq!(found.names, ["a", "b"]);
}
