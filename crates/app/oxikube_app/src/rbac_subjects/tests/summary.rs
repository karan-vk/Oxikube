//! The Users and Groups pages and the lookup of a typed name.

use oxikube_domain::rbac::Subject;

use super::{NS, group, handle, user};
use crate::rbac_subjects::{SubjectList, resolve_subject, summaries};

#[test]
fn users_list_users_and_service_accounts_with_their_binding_and_role_counts() {
    let rows = handle().read(|index| summaries(index, SubjectList::Users));
    let names: Vec<String> = rows.iter().map(|r| r.name().to_string()).collect();
    assert!(names.contains(&"fx-viewer".to_owned()), "{names:?}");
    assert!(names.contains(&"fx-root".to_owned()), "{names:?}");
    // A ServiceAccount is listed under its authenticating identity.
    assert!(
        names.contains(&format!("system:serviceaccount:{NS}:fx-sa")),
        "{names:?}"
    );
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "sorted: {names:?}");
    let viewer = rows
        .iter()
        .find(|r| r.subject == user("fx-viewer"))
        .unwrap();
    assert_eq!((viewer.bindings, viewer.roles), (1, 1));
    // Groups are not users.
    assert!(rows.iter().all(|r| !matches!(r.subject, Subject::Group(_))));
}

#[test]
fn groups_list_only_groups_named_by_a_binding() {
    let rows = handle().read(|index| summaries(index, SubjectList::Groups));
    let names: Vec<String> = rows.iter().map(|r| r.name().to_string()).collect();
    assert!(names.contains(&"fx-team".to_owned()), "{names:?}");
    assert!(
        names.contains(&"system:authenticated".to_owned()),
        "{names:?}"
    );
    // `system:authenticated` is bound by three ClusterRoleBindings.
    let authenticated = rows
        .iter()
        .find(|r| r.subject == group("system:authenticated"))
        .unwrap();
    assert_eq!(authenticated.bindings, 3);
    assert!(rows.iter().all(|r| matches!(r.subject, Subject::Group(_))));
}

#[test]
fn a_typed_name_resolves_to_the_user_a_service_account_or_the_group_it_is() {
    let handle = handle();
    handle.read(|index| {
        assert_eq!(resolve_subject(index, "fx-viewer"), user("fx-viewer"));
        assert_eq!(
            resolve_subject(index, &format!("system:serviceaccount:{NS}:fx-sa")),
            Subject::service_account(NS, "fx-sa")
        );
        assert_eq!(resolve_subject(index, "fx-team"), group("fx-team"));
        // No binding names it: it is a user nobody has granted anything.
        assert_eq!(resolve_subject(index, "nobody"), user("nobody"));
    });
}
