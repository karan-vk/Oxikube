//! Random Role and binding sets: the index answers `can` exactly as a naive reference
//! authoriser over the objects as written, whether it was built from snapshots or one insert
//! at a time.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::access::{Access, non_resource_url_matches};
use oxikube_domain::ids::Gvr;
use oxikube_domain::rbac::{Binding, GrantScope, PolicyRule, RbacKind, Role, RoleKind, Subject};
use proptest::prelude::*;
use proptest::sample::subsequence;
use serde_json::{Value, json};

use super::{index_of, role};
use crate::access::SubjectIndex;
use crate::store::{ObjectKey, RowChange, RowOp, StoreObject};

const NAMESPACES: [&str; 2] = ["a", "b"];

fn rules() -> impl Strategy<Value = Value> {
    let rule = prop_oneof![
        4 => (
            subsequence(&["get", "list", "delete", "bind", "*"][..], 1..=2),
            subsequence(&["", "apps", "*"][..], 1..=2),
            subsequence(&["pods", "pods/log", "deployments", "*/scale", "*"][..], 1..=2),
            prop_oneof![3 => Just(Vec::new()), 1 => subsequence(&["x", "y"][..], 1..=1)],
        )
            .prop_map(|(verbs, groups, resources, names)| json!({
                "verbs": verbs, "apiGroups": groups, "resources": resources, "resourceNames": names,
            })),
        1 => subsequence(&["/healthz", "/metrics/*"][..], 1..=2)
            .prop_map(|urls| json!({"verbs": ["get"], "nonResourceURLs": urls})),
    ];
    prop::collection::vec(rule, 0..3).prop_map(Value::from)
}

fn subjects() -> impl Strategy<Value = Value> {
    subsequence(
        &[
            ("User", "alice", None),
            ("User", "bob", None),
            ("Group", "devs", None),
            ("Group", "system:authenticated", None),
            ("Group", "system:serviceaccounts:a", None),
            ("ServiceAccount", "builder", Some("a")),
            ("User", "system:serviceaccount:b:x", None),
        ][..],
        1..=3,
    )
    .prop_map(|items| {
        Value::from(
            items
                .into_iter()
                .map(|(kind, name, namespace)| json!({"kind": kind, "name": name, "namespace": namespace}))
                .collect::<Vec<_>>(),
        )
    })
}

/// Up to four ClusterRoles (`cr3` aggregates those labelled `agg`), two Roles per namespace,
/// and up to six bindings of every shape (including a ClusterRoleBinding naming a Role, which
/// grants nothing).
fn policy() -> impl Strategy<Value = Vec<Resource>> {
    let cluster_roles = prop::collection::vec((rules(), any::<bool>()), 4).prop_map(|roles| {
        roles
            .into_iter()
            .enumerate()
            .map(|(at, (rules, aggregated_into))| {
                let labels = if aggregated_into {
                    json!({"agg": "true"})
                } else {
                    json!({})
                };
                let mut object = role(None, &format!("cr{at}"), rules, labels).to_value();
                if at == 3 {
                    object["aggregationRule"] =
                        json!({"clusterRoleSelectors": [{"matchLabels": {"agg": "true"}}]});
                }
                Resource::from_json(object).expect("a ClusterRole")
            })
            .collect::<Vec<_>>()
    });
    let roles = prop::collection::vec(rules(), 4).prop_map(|all| {
        all.into_iter()
            .enumerate()
            .map(|(at, rules)| {
                role(
                    Some(NAMESPACES[at % 2]),
                    &format!("r{}", at / 2),
                    rules,
                    json!({}),
                )
            })
            .collect::<Vec<_>>()
    });
    let binding = (0..3usize, 0..4usize, any::<bool>(), subjects());
    let bindings = prop::collection::vec(binding, 0..6).prop_map(|all| {
        all.into_iter()
            .enumerate()
            .map(|(at, (where_at, role_at, to_cluster_role, subjects))| {
                let namespace = NAMESPACES.get(where_at).copied();
                let role_ref = if to_cluster_role {
                    ("ClusterRole", format!("cr{role_at}"))
                } else {
                    ("Role", format!("r{}", role_at % 2))
                };
                super::binding(
                    namespace,
                    &format!("b{at}"),
                    (role_ref.0, &role_ref.1),
                    subjects,
                )
            })
            .collect::<Vec<_>>()
    });
    (cluster_roles, roles, bindings).prop_map(|(mut all, roles, bindings)| {
        all.extend(roles);
        all.extend(bindings);
        all
    })
}

/// The API server's authoriser, naively: the rules as written of every binding that names the
/// requester and holds in `namespace` (plus, for an aggregated ClusterRole, the rules of the
/// ClusterRoles it selects). URLs come only from cluster-wide bindings.
fn applicable(objects: &[Resource], subject: &Subject, namespace: Option<&str>) -> Vec<PolicyRule> {
    let roles: Vec<Role> = objects
        .iter()
        .filter_map(|o| Role::from_resource(o).ok())
        .collect();
    let info = subject.user_info();
    let mut rules = Vec::new();
    for binding in objects
        .iter()
        .filter_map(|o| Binding::from_resource(o).ok())
    {
        if !binding.subjects.iter().any(|s| info.is(s)) || !binding.scope().covers(namespace) {
            continue;
        }
        let Some(id) = binding.role_id() else {
            continue;
        };
        let Some(own) = roles.iter().find(|r| r.id == id) else {
            continue;
        };
        let mut sources = vec![own];
        if let Some(aggregation) = &own.aggregation {
            sources.extend(roles.iter().filter(|r| {
                r.id.kind == RoleKind::ClusterRole
                    && r.id != own.id
                    && aggregation.selects(&r.labels)
            }));
        }
        let cluster_wide = binding.scope() == GrantScope::Cluster;
        for rule in sources.iter().flat_map(|role| &role.rules) {
            let mut rule = rule.clone();
            if !cluster_wide {
                rule.non_resource_urls.clear();
            }
            rules.push(rule);
        }
    }
    rules
}

fn reference(
    rules: &[PolicyRule],
    (verb, group, resource): (&str, &str, &str),
    name: Option<&str>,
) -> Access {
    rules
        .iter()
        .filter_map(|rule| rule.to_access_rule().level_for(verb, group, resource, name))
        .max()
        .unwrap_or(Access::Denied)
}

fn reference_url(rules: &[PolicyRule], path: &str) -> bool {
    rules.iter().any(|rule| {
        rule.verbs.iter().any(|v| v == "get" || v == "*")
            && rule
                .non_resource_urls
                .iter()
                .any(|url| non_resource_url_matches(url, path))
    })
}

/// The same objects inserted one at a time, each at its sorted position, kinds interleaved.
fn index_by_inserts(objects: &[Resource]) -> SubjectIndex {
    let mut index = SubjectIndex::new();
    let mut rows: Vec<(RbacKind, Vec<ObjectKey>)> =
        RbacKind::ALL.iter().map(|k| (*k, Vec::new())).collect();
    for object in objects.iter().rev() {
        let kind = RbacKind::of(&object.kind).expect("an RBAC object");
        let keys = &mut rows
            .iter_mut()
            .find(|(k, _)| *k == kind)
            .expect("every kind has rows")
            .1;
        let key = ObjectKey::of(&object.meta);
        let at = keys.partition_point(|k| *k < key);
        keys.insert(at, key);
        let op = RowOp::Insert {
            index: at,
            object: Arc::new(StoreObject::Resource(object.clone())),
        };
        index.apply_rows(kind, &RowChange::Ops(vec![op]));
    }
    index
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn the_index_answers_like_the_reference_authoriser(objects in policy()) {
        let snapshot = index_of(objects.clone());
        let inserted = index_by_inserts(&objects);
        let asked = [
            Subject::User("alice".into()),
            Subject::User("bob".into()),
            Subject::Group("devs".into()),
            Subject::service_account("a", "builder"),
            Subject::service_account("b", "x"),
            Subject::User("nobody".into()),
        ];
        let requests = [
            ("get", "", "pods"),
            ("list", "", "pods/log"),
            ("delete", "apps", "deployments"),
            ("get", "apps", "deployments/scale"),
            ("bind", "x.io", "pods"),
            ("patch", "", "pods"),
        ];
        for subject in &asked {
            prop_assert_eq!(snapshot.rules_for(subject), inserted.rules_for(subject));
            for namespace in [None, Some("a"), Some("b")] {
                let rules = applicable(&objects, subject, namespace);
                for (verb, group, resource) in requests {
                    for name in [None, Some("x"), Some("z")] {
                        let gvr = Gvr::new(group, "v1", resource);
                        prop_assert_eq!(
                            snapshot.can(subject, verb, &gvr, namespace, name),
                            reference(&rules, (verb, group, resource), name),
                            "{} {} {}/{} {:?} in {:?}", subject, verb, group, resource, name, namespace
                        );
                    }
                }
            }
            let cluster_rules = applicable(&objects, subject, None);
            for path in ["/healthz", "/metrics", "/metrics/x"] {
                prop_assert_eq!(
                    snapshot.can_url(subject, "get", path),
                    reference_url(&cluster_rules, path),
                    "{} {}", subject, path
                );
            }
        }
    }
}
