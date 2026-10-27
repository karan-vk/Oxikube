//! The k9s-style matrix projection.

use super::super::{
    EXTRAS_HEADER, MATRIX_VERBS, NormalisedRules, PolicyMatrix, PolicyRule, verb_header,
};
use crate::access::Access;
use crate::kinds::Verb;

#[test]
fn rows_per_group_and_resource_with_k9s_columns() {
    let headers: Vec<&str> = MATRIX_VERBS.iter().map(|verb| verb_header(*verb)).collect();
    assert_eq!(
        headers,
        [
            "GET", "LIST", "WATCH", "CREATE", "PATCH", "UPDATE", "DELETE", "DEL-LIST"
        ]
    );
    assert_eq!(EXTRAS_HEADER, "EXTRAS");

    let rules = NormalisedRules::from_policy(&[
        PolicyRule::resource(&["get", "list", "watch"], &["apps"], &["*"], &[]),
        PolicyRule::resource(&["delete", "patch"], &["apps"], &["deployments"], &[]),
        PolicyRule::resource(&["get"], &[""], &["secrets"], &["s"]),
        PolicyRule::resource(
            &["bind", "escalate"],
            &["rbac.authorization.k8s.io"],
            &["roles"],
            &[],
        ),
        PolicyRule::non_resource(&["get"], &["/healthz"]),
    ]);
    let matrix = PolicyMatrix::from_rules(&rules);
    let keys: Vec<(&str, &str)> = matrix
        .rows
        .iter()
        .map(|row| (row.api_group.as_str(), row.resource.as_str()))
        .collect();
    assert_eq!(
        keys,
        [
            ("", "secrets"),
            ("apps", "*"),
            ("apps", "deployments"),
            ("rbac.authorization.k8s.io", "roles"),
        ]
    );
    let deployments = &matrix.rows[2];
    assert_eq!(
        deployments.cell(Verb::Get),
        Access::Granted,
        "from the * row"
    );
    assert_eq!(deployments.cell(Verb::Delete), Access::Granted);
    assert_eq!(deployments.cell(Verb::Patch), Access::Granted);
    assert_eq!(deployments.cell(Verb::Create), Access::Denied);
    assert!(deployments.extras.is_empty());
    assert_eq!(
        matrix.rows[0].cell(Verb::Get),
        Access::Restricted,
        "named Secret only"
    );
    assert_eq!(matrix.rows[3].extras, ["bind", "escalate"]);
    assert_eq!(matrix.non_resource.len(), 1);
}

#[test]
fn every_verb_fills_every_column_and_shows_as_an_extra() {
    let matrix = PolicyMatrix::from_rules(&NormalisedRules::from_policy(&[PolicyRule::resource(
        &["*"],
        &["*"],
        &["*"],
        &[],
    )]));
    assert_eq!(matrix.rows.len(), 1);
    assert!(
        matrix.rows[0]
            .cells
            .iter()
            .all(|cell| *cell == Access::Granted)
    );
    assert_eq!(matrix.rows[0].extras, ["*"]);
    assert_eq!(
        PolicyMatrix::from_rules(&NormalisedRules::default()),
        PolicyMatrix::default()
    );
}
