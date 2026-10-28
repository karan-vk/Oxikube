//! The kind suites' RBAC scenario (E17-S10, `oxikube_testkit::fixtures::rbac::RbacScenario`)
//! indexed without a cluster: the objects the kind tests apply, over the recorded cluster
//! defaults (`rbac::cluster()`, a v1.37 kind cluster's built-in roles and bindings). It pins what
//! the live tests expect, so a fixture that drifts fails here first, without kind.

use oxikube_domain::Resource;
use oxikube_domain::access::Access;
use oxikube_domain::ids::Gvr;
use oxikube_domain::rbac::{NormalisedRules, PolicyRule, RbacKind, Subject};
use oxikube_testkit::fixtures::rbac::{self, POD_VERBS, RbacScenario, printed_rules};

use super::index_of;
use crate::access::SubjectIndex;

fn scenario() -> RbacScenario {
    RbacScenario::new("oxi-test-0000-home", "oxi-test-0000-other")
}

/// The recorded cluster plus the scenario's RBAC objects (its namespaces and account are not
/// policy), and the view binding when `granted`.
fn index(scenario: &RbacScenario, granted: bool) -> SubjectIndex {
    let mut objects = scenario.objects();
    if granted {
        objects.push(scenario.view_binding());
    }
    let policy = objects
        .into_iter()
        .map(|object| Resource::from_json(object).expect("a scenario object"))
        .filter(|object| RbacKind::of(&object.kind).is_some());
    index_of(rbac::cluster().into_iter().chain(policy))
}

/// What `kubectl auth can-i --list` printed for `subject` in `namespace` when recorded.
fn recorded(subject: &Subject, namespace: &str) -> NormalisedRules {
    let review = rbac::reviews()
        .into_iter()
        .find(|review| &review.subject == subject && review.namespace == namespace)
        .unwrap_or_else(|| panic!("a recorded review of {subject} in {namespace}"));
    NormalisedRules::from_policy(&printed_rules(&review.can_i_list))
}

/// Everything a ServiceAccount with no binding of its own may do anywhere: the recorded
/// `fx-sa` in `default`, where nothing names it.
fn account_defaults() -> NormalisedRules {
    recorded(&Subject::service_account("oxi-rbac-fx", "fx-sa"), "default")
}

/// The built-in `view` in a namespace, as the recorded `fx-viewer` (bound to it there) has it.
fn view() -> NormalisedRules {
    recorded(&Subject::User("fx-viewer".into()), "oxi-rbac-fx")
}

fn pods() -> NormalisedRules {
    NormalisedRules::from_policy(&[PolicyRule::resource(&POD_VERBS, &[""], &["pods"], &[])])
}

#[test]
fn the_account_reads_pods_at_home_and_nothing_elsewhere() {
    let scenario = scenario();
    let index = index(&scenario, false);
    let account = scenario.account();
    assert_eq!(
        index.rules_in(&account, Some(&scenario.home)),
        NormalisedRules::union([&account_defaults(), &pods()]),
    );
    assert_eq!(
        index.rules_in(&account, Some(&scenario.other)),
        account_defaults()
    );
    let can = |verb: &str, group: &str, resource: &str, namespace: Option<&str>| {
        let gvr = Gvr::new(group, "v1", resource);
        index.can(&account, verb, &gvr, namespace, None)
    };
    assert_eq!(
        can("list", "", "pods", Some(&scenario.home)),
        Access::Granted
    );
    assert_eq!(
        can("delete", "", "pods", Some(&scenario.home)),
        Access::Denied
    );
    assert_eq!(
        can("list", "", "pods", Some(&scenario.other)),
        Access::Denied
    );
    // The sections the sidebar hides for it: Config (secrets, config maps) and the nodes.
    assert_eq!(
        can("list", "", "secrets", Some(&scenario.home)),
        Access::Denied
    );
    assert_eq!(
        can("list", "", "configmaps", Some(&scenario.home)),
        Access::Denied
    );
    assert_eq!(can("list", "", "nodes", None), Access::Denied);
}

#[test]
fn the_view_binding_adds_the_aggregated_view_role_in_the_other_namespace_only() {
    let scenario = scenario();
    let index = index(&scenario, true);
    let account = scenario.account();
    assert_eq!(
        index.rules_in(&account, Some(&scenario.other)),
        NormalisedRules::union([&account_defaults(), &view()]),
    );
    assert_eq!(
        index.rules_in(&account, Some(&scenario.home)),
        NormalisedRules::union([&account_defaults(), &pods()]),
        "nothing changes at home"
    );
    let configmaps = Gvr::new("", "v1", "configmaps");
    let secrets = Gvr::new("", "v1", "secrets");
    let other = Some(scenario.other.as_str());
    assert_eq!(
        index.can(&account, "list", &configmaps, other, None),
        Access::Granted
    );
    // `view` leaves Secrets out.
    assert_eq!(
        index.can(&account, "list", &secrets, other, None),
        Access::Denied
    );
}
