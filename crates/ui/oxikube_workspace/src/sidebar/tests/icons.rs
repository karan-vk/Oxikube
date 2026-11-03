//! The kind behind each entry row, which picks its icon through the icon theme (E22-S11).

use std::collections::BTreeMap;

use oxikube_app::{AccessOutcome, CustomKind, CustomResourceGroup};
use oxikube_domain::access::AccessRules;

use crate::sidebar::{AccessState, EntryKind, Row, RowInputs, build_rows, core_sections};

fn entry_kinds(custom: &[CustomResourceGroup]) -> BTreeMap<String, Option<EntryKind>> {
    let open = BTreeMap::from([("crd:example.com".to_owned(), true)]);
    build_rows(&RowInputs {
        sections: &core_sections(),
        integrations: &[],
        custom: Some(custom),
        access: &AccessState::Known(AccessOutcome::Reviewed(AccessRules::all_access())),
        crd_watch_forbidden: false,
        open: &open,
        custom_query: "",
        show_restricted: false,
        gate: None,
    })
    .into_iter()
    .filter_map(|row| match row {
        Row::Entry(entry) => Some((entry.id.to_string(), entry.kind)),
        _ => None,
    })
    .collect()
}

fn kind(group: &str, kind: &str) -> Option<EntryKind> {
    Some(EntryKind {
        group: group.into(),
        kind: kind.into(),
    })
}

#[test]
fn kind_entries_carry_their_kind_and_the_rest_none() {
    let custom = [CustomResourceGroup {
        group: "example.com".into(),
        kinds: vec![CustomKind {
            kind: "Widget".into(),
            plural: "widgets".into(),
            version: "v1".into(),
            namespaced: true,
        }],
    }];
    let kinds = entry_kinds(&custom);
    assert_eq!(kinds["workloads/pods"], kind("", "Pod"));
    assert_eq!(kinds["workloads/deployments"], kind("apps", "Deployment"));
    assert_eq!(kinds["nodes/nodes"], kind("", "Node"));
    assert_eq!(
        kinds["network/ingresses"],
        kind("networking.k8s.io", "Ingress")
    );
    assert_eq!(
        kinds["custom-resources/definitions"],
        kind("apiextensions.k8s.io", "CustomResourceDefinition")
    );
    assert_eq!(
        kinds["crd:example.com/widgets"],
        kind("example.com", "Widget")
    );
    // Pages and commands list no kind: an empty icon slot.
    assert_eq!(kinds["workloads/workloads-overview"], None);
    assert_eq!(kinds["helm/releases"], None);
    assert_eq!(kinds["access-control/rbac-users"], None);
}
