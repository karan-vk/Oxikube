//! The per-verb matrix of a subject, its provenance and the recorded `can-i` reviews.

use oxikube_domain::access::{Access, AccessRules};
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{
    BindingKind, GrantScope, NormalisedRules, PolicyRule, RoleKind, Subject,
};
use oxikube_testkit::fixtures::rbac;
use serde_json::Value;

use super::{NS, group, handle, line, namespace, user};
use crate::rbac_subjects::{
    Inherit, LineKind, MatrixColumn, provenance, scopes_of, subject_matrix,
};

const READ: [Verb; 3] = [Verb::Get, Verb::List, Verb::Watch];

#[test]
fn view_reads_but_cannot_write() {
    let matrix = handle()
        .read(|index| subject_matrix(index, &user("fx-viewer"), &namespace(NS), Inherit::Direct));
    assert_eq!(matrix.grants, 1);
    let pods = line(&matrix.lines, "", "pods");
    for verb in READ {
        assert_eq!(pods.cell(verb), Access::Granted, "{verb:?}");
    }
    for verb in [
        Verb::Create,
        Verb::Patch,
        Verb::Update,
        Verb::Delete,
        Verb::DeleteCollection,
    ] {
        assert_eq!(pods.cell(verb), Access::Denied, "{verb:?}");
    }
    assert!(pods.extras.is_empty());
}

#[test]
fn edit_writes_and_admin_adds_roles_but_neither_is_cluster_wide() {
    let handle = handle();
    handle.read(|index| {
        let edit = subject_matrix(index, &user("fx-editor"), &namespace(NS), Inherit::Direct);
        let pods = line(&edit.lines, "", "pods");
        for verb in [
            Verb::Create,
            Verb::Patch,
            Verb::Update,
            Verb::Delete,
            Verb::DeleteCollection,
        ] {
            assert_eq!(pods.cell(verb), Access::Granted, "edit {verb:?}");
        }
        let admin = subject_matrix(index, &user("fx-admin"), &namespace(NS), Inherit::Direct);
        let roles = line(&admin.lines, "rbac.authorization.k8s.io", "roles");
        assert_eq!(roles.cell(Verb::Create), Access::Granted);
        // A RoleBinding gives nothing outside its namespace.
        let cluster = subject_matrix(
            index,
            &user("fx-admin"),
            &GrantScope::Cluster,
            Inherit::Direct,
        );
        assert!(cluster.is_empty());
        assert_eq!(cluster.grants, 0);
    });
}

#[test]
fn cluster_admin_is_every_verb_on_every_resource_and_url() {
    let matrix = handle().read(|index| {
        subject_matrix(
            index,
            &user("fx-root"),
            &GrantScope::Cluster,
            Inherit::Direct,
        )
    });
    let all = line(&matrix.lines, "*", "*");
    assert!(all.cells.iter().all(|c| *c == Access::Granted));
    assert_eq!(all.extras, ["*"]);
    let urls: Vec<_> = matrix
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Url)
        .collect();
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].resource, "*");
    assert_eq!(urls[0].cell(Verb::Get), Access::Granted);
}

#[test]
fn resource_names_show_as_partial_and_urls_as_rows() {
    let matrix = handle().read(|index| {
        subject_matrix(
            index,
            &group("fx-team"),
            &GrantScope::Cluster,
            Inherit::Direct,
        )
    });
    let configmaps = line(&matrix.lines, "", "configmaps");
    assert_eq!(configmaps.cell(Verb::Get), Access::Restricted);
    assert_eq!(configmaps.cell(Verb::Update), Access::Restricted);
    assert_eq!(configmaps.cell(Verb::Delete), Access::Denied);
    // Aggregated from the ClusterRoles the selector collects.
    let deployments = line(&matrix.lines, "apps", "deployments");
    assert_eq!(deployments.cell(Verb::Watch), Access::Granted);
    let health = matrix
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Url && l.resource == "/healthz")
        .expect("the /healthz row");
    assert_eq!(health.cell(Verb::Get), Access::Granted);
    assert_eq!(health.group_label(), "-");
}

#[test]
fn a_cell_names_the_bindings_and_roles_that_grant_it() {
    let handle = handle();
    handle.read(|index| {
        let scope = namespace(NS);
        let matrix = subject_matrix(index, &user("fx-viewer"), &scope, Inherit::Direct);
        let pods = line(&matrix.lines, "", "pods");
        let why = provenance(
            index,
            &user("fx-viewer"),
            &scope,
            Inherit::Direct,
            pods,
            MatrixColumn::Verb(Verb::Get),
        );
        assert_eq!(why.len(), 1);
        assert_eq!(why[0].binding.kind, BindingKind::RoleBinding);
        assert_eq!(&*why[0].binding.name, "fx-viewer-view");
        assert_eq!(why[0].role.kind, RoleKind::ClusterRole);
        assert_eq!(&*why[0].role.name, "view");
        assert!(why[0].role_found);
        assert_eq!(why[0].access, Access::Granted);
        // Nothing grants a delete.
        assert!(
            provenance(
                index,
                &user("fx-viewer"),
                &scope,
                Inherit::Direct,
                pods,
                MatrixColumn::Verb(Verb::Delete),
            )
            .is_empty()
        );
    });
}

#[test]
fn a_partial_cell_lists_the_names_it_is_limited_to() {
    handle().read(|index| {
        let matrix = subject_matrix(
            index,
            &group("fx-team"),
            &GrantScope::Cluster,
            Inherit::Direct,
        );
        let configmaps = line(&matrix.lines, "", "configmaps");
        let why = provenance(
            index,
            &group("fx-team"),
            &GrantScope::Cluster,
            Inherit::Direct,
            configmaps,
            MatrixColumn::Verb(Verb::Get),
        );
        assert_eq!(why.len(), 1);
        assert_eq!(why[0].access, Access::Restricted);
        assert_eq!(why[0].names, ["team-flags", "team-settings"]);
    });
}

#[test]
fn implicit_groups_are_a_toggle_and_the_cell_says_which_group_gave_it() {
    let handle = handle();
    handle.read(|index| {
        let account = Subject::service_account(NS, "fx-sa");
        let scope = namespace(NS);
        let direct = subject_matrix(index, &account, &scope, Inherit::Direct);
        assert_eq!(direct.grants, 1, "only fx-sa-role names the account");
        assert!(direct.lines.iter().all(|l| l.resource != "events"));

        let inherited = subject_matrix(index, &account, &scope, Inherit::ImplicitGroups);
        assert!(inherited.grants > direct.grants);
        // `fx-namespace-sas` binds the namespace's ServiceAccount group to a role with `events`.
        let events = line(&inherited.lines, "", "events");
        assert_eq!(events.cell(Verb::List), Access::Granted);
        let why = provenance(
            index,
            &account,
            &scope,
            Inherit::ImplicitGroups,
            events,
            MatrixColumn::Verb(Verb::List),
        );
        assert!(
            why.iter()
                .any(|p| p.via == group(&format!("system:serviceaccounts:{NS}"))),
            "{why:?}"
        );
    });
}

#[test]
fn a_subject_no_binding_names_has_an_empty_matrix() {
    handle().read(|index| {
        let matrix = subject_matrix(index, &user("nobody"), &namespace(NS), Inherit::Direct);
        assert!(matrix.is_empty());
        assert_eq!(matrix.grants, 0);
    });
}

#[test]
fn the_scopes_are_the_cluster_and_each_namespace_the_subject_is_bound_in() {
    handle().read(|index| {
        assert_eq!(
            scopes_of(index, &user("fx-viewer"), Inherit::Direct),
            [GrantScope::Cluster, namespace(NS)]
        );
        assert_eq!(
            scopes_of(index, &user("nobody"), Inherit::Direct),
            [GrantScope::Cluster]
        );
    });
}

#[test]
fn the_export_is_tab_separated_with_a_header() {
    handle().read(|index| {
        let matrix = subject_matrix(index, &user("fx-viewer"), &namespace(NS), Inherit::Direct);
        let text = matrix.to_tsv();
        let mut rows = text.lines();
        assert_eq!(
            rows.next(),
            Some("API GROUP\tRESOURCE\tGET\tLIST\tWATCH\tCREATE\tPATCH\tUPDATE\tDELETE\tDEL-LIST\tEXTRAS")
        );
        let pods = rows.find(|row| row.starts_with("core\tpods\t")).unwrap();
        assert_eq!(pods, "core\tpods\tyes\tyes\tyes\t-\t-\t-\t-\t-\t-");
        assert!(text.lines().all(|row| row.split('\t').count() == 11), "{text}");
    });
}

fn rules_of(status: &Value, key: &str) -> Vec<PolicyRule> {
    serde_json::from_value(status[key].clone()).expect("review rules are PolicyRules")
}

/// What `kubectl auth can-i --list --as=<subject> -n <ns>` answered is what the matrix of that
/// subject, with the implicit groups, shows for every row and verb.
#[test]
fn the_matrix_agrees_with_every_recorded_rules_review() {
    let handle = handle();
    let reviews = rbac::reviews();
    assert!(!reviews.is_empty());
    for review in reviews {
        let theirs: AccessRules =
            NormalisedRules::from_policy(&rules_of(&review.status, "resourceRules"))
                .to_access_rules();
        handle.read(|index| {
            let scope = namespace(&review.namespace);
            let ours = subject_matrix(index, &review.subject, &scope, Inherit::ImplicitGroups);
            for line in ours.lines.iter().filter(|l| l.kind == LineKind::Resource) {
                for verb in [
                    Verb::Get,
                    Verb::List,
                    Verb::Watch,
                    Verb::Create,
                    Verb::Patch,
                    Verb::Update,
                    Verb::Delete,
                    Verb::DeleteCollection,
                ] {
                    let expected = theirs.level(verb.as_str(), &line.api_group, &line.resource);
                    assert_eq!(
                        line.cell(verb),
                        expected,
                        "{} in {}: {} {}/{}",
                        review.subject,
                        review.namespace,
                        verb.as_str(),
                        line.api_group,
                        line.resource
                    );
                }
            }
        });
    }
}
