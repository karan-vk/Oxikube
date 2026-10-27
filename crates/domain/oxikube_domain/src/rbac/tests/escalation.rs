use std::sync::Arc;

use super::super::{Escalation, RoleKind, RoleRef, Subject};

fn cluster_role(name: &str) -> RoleRef {
    RoleRef {
        kind: RoleKind::ClusterRole,
        name: Arc::from(name),
    }
}

#[test]
fn cluster_admin_is_singled_out_but_not_a_role_of_that_name() {
    let user = [Subject::User("ana".into())];
    assert_eq!(
        Escalation::of(&cluster_role("cluster-admin"), &user),
        [Escalation::ClusterAdmin]
    );
    assert!(Escalation::of(&cluster_role("view"), &user).is_empty());
    let role = RoleRef {
        kind: RoleKind::Role,
        name: Arc::from("cluster-admin"),
    };
    assert!(Escalation::of(&role, &user).is_empty());
}

#[test]
fn everybody_groups_and_anonymous_are_broad() {
    for subject in [
        Subject::Group("system:authenticated".into()),
        Subject::Group("system:unauthenticated".into()),
        Subject::Group("system:serviceaccounts".into()),
        Subject::User("system:anonymous".into()),
    ] {
        assert_eq!(
            Escalation::of(&cluster_role("view"), &[subject]),
            [Escalation::BroadSubject]
        );
    }
    let narrow = [
        Subject::Group("team".into()),
        Subject::service_account("demo", "builder"),
    ];
    assert!(Escalation::of(&cluster_role("view"), &narrow).is_empty());
    let both = [Subject::Group("system:authenticated".into())];
    assert_eq!(
        Escalation::of(&cluster_role("cluster-admin"), &both),
        [Escalation::ClusterAdmin, Escalation::BroadSubject]
    );
}
