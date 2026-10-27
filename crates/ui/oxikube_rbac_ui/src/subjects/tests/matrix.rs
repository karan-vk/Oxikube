//! A subject's matrix page.

use gpui::TestAppContext;
use oxikube_app::rbac_subjects::{Inherit, MatrixColumn};
use oxikube_domain::access::Access;
use oxikube_domain::command::Command;
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{GrantScope, Subject};

use super::{Fx, NS, cluster};
use crate::subjects::Page;

fn row_of(
    fx: &mut Fx,
    view: &gpui::Entity<crate::subjects::SubjectsView>,
    group: &str,
    resource: &str,
) -> usize {
    fx.vcx.update(|_, cx| {
        view.read(cx).grid().read(cx, |d| {
            (0..d.len())
                .find(|&row| {
                    d.line(row)
                        .is_some_and(|l| l.api_group == group && l.resource == resource)
                })
                .unwrap_or_else(|| panic!("no row {group}/{resource}: {:?}", d.shown_keys()))
        })
    })
}

#[gpui::test]
fn a_users_matrix_shows_the_namespace_it_is_bound_in(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.subject("fx-viewer");
    fx.vcx.update(|_, cx| {
        let page = view.read(cx);
        assert_eq!(page.page(), Page::Subject);
        // No cluster-wide binding, so the page opens on the namespace that has one.
        assert_eq!(page.scope(), &GrantScope::Namespace(NS.into()));
        let matrix = page.matrix().expect("computed");
        assert_eq!(matrix.grants, 1);
    });
    let pods = row_of(&mut fx, &view, "", "pods");
    fx.vcx.update(|_, cx| {
        let line = view
            .read(cx)
            .grid()
            .read(cx, |d| d.line(pods).cloned().unwrap());
        assert_eq!(line.cell(Verb::Get), Access::Granted);
        assert_eq!(line.cell(Verb::Delete), Access::Denied);
    });
    assert!(fx.drawn("rbac-matrix"));
    assert!(fx.drawn("rbac-scope-0"));
    assert!(fx.drawn("rbac-scope-1"));
}

#[gpui::test]
fn selecting_a_row_lists_the_bindings_and_roles_behind_each_verb(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.subject("fx-viewer");
    let pods = row_of(&mut fx, &view, "", "pods");
    fx.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.select_row(pods, cx)));
    fx.settle();
    fx.vcx.update(|_, cx| {
        let why = view.read(cx).provenance();
        assert_eq!(why.len(), 1, "{why:?}");
        assert_eq!(&*why[0].binding.name, "fx-viewer-view");
        assert_eq!(&*why[0].role.name, "view");
    });
    assert!(fx.drawn("rbac-why"));
    assert!(fx.drawn("rbac-why-0"));
    // A verb nothing grants says so.
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_column(MatrixColumn::Verb(Verb::Delete), cx)
        });
    });
    fx.settle();
    assert!(fx.drawn("rbac-why-none"));
}

#[gpui::test]
fn the_scope_and_implicit_groups_toggles_recompute_the_matrix(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.subject("fx-viewer");
    fx.click("rbac-scope-0");
    fx.vcx.update(|_, cx| {
        let page = view.read(cx);
        assert_eq!(page.scope(), &GrantScope::Cluster);
        assert!(
            page.matrix().unwrap().is_empty(),
            "nothing is bound cluster-wide"
        );
    });
    fx.click("rbac-scope-1");
    fx.vcx
        .update(|_, cx| assert!(!view.read(cx).matrix().unwrap().is_empty()));

    fx.click("rbac-implicit");
    fx.vcx.update(|_, cx| {
        let matrix = view.read(cx).matrix().unwrap().clone();
        assert_eq!(matrix.inherit, Inherit::ImplicitGroups);
        // `system:authenticated` adds its three ClusterRoleBindings' grants.
        assert!(matrix.grants > 1, "{}", matrix.grants);
    });
}

#[gpui::test]
fn a_group_says_it_has_no_members_to_show_and_a_service_account_keeps_its_identity(
    cx: &mut TestAppContext,
) {
    let mut fx = Fx::new(cx);
    let team = fx.subject("fx-team");
    fx.vcx.update(|_, cx| {
        assert_eq!(
            team.read(cx).subject(),
            Some(&Subject::Group("fx-team".into()))
        );
        // Cluster-wide: partial cells for the named ConfigMaps.
        let matrix = team.read(cx).matrix().unwrap().clone();
        assert!(
            matrix
                .lines
                .iter()
                .any(|l| l.resource == "configmaps" && l.cell(Verb::Get) == Access::Restricted)
        );
    });
    assert!(fx.drawn("rbac-group-note"));

    let sa = fx.subject(&format!("system:serviceaccount:{NS}:fx-sa"));
    fx.vcx.update(|_, cx| {
        assert_eq!(
            sa.read(cx).subject(),
            Some(&Subject::service_account(NS, "fx-sa"))
        );
    });
}

#[gpui::test]
fn an_unknown_subject_gets_an_empty_state_with_a_hint(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.subject("nobody-at-all");
    fx.vcx.update(|_, cx| {
        let page = view.read(cx);
        assert!(page.matrix().unwrap().is_empty());
    });
    assert!(fx.drawn("rbac-matrix-empty"));
}

#[gpui::test]
fn the_filter_narrows_the_matrix_rows(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.subject("fx-viewer");
    fx.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.set_filter_text("^pods$", cx)));
    fx.settle();
    fx.vcx.update(|_, cx| {
        let keys = view.read(cx).grid().read(cx, |d| d.shown_keys());
        assert_eq!(keys, ["/pods", "metrics.k8s.io/pods"]);
    });
}

#[gpui::test]
fn copying_sends_the_command_and_puts_the_matrix_on_the_clipboard(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let _view = fx.subject("fx-viewer");
    fx.click("rbac-copy");
    assert!(
        fx.dispatcher
            .sent()
            .contains(&Command::RbacCopyMatrix { cluster: cluster() })
    );
    let text = fx
        .vcx
        .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .expect("the matrix was copied");
    assert!(
        text.starts_with("API GROUP\tRESOURCE\tGET\tLIST\tWATCH\tCREATE\tPATCH\tUPDATE\tDELETE\tDEL-LIST\tEXTRAS\n"),
        "{text}"
    );
    assert!(text.contains("core\tpods\tyes\tyes\tyes\t-"), "{text}");
}

#[gpui::test]
fn back_returns_to_the_list_the_subject_came_from(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.subject("fx-team");
    fx.click("rbac-back");
    fx.vcx
        .update(|_, cx| assert_eq!(view.read(cx).page(), Page::Groups));
    let view = fx.subject("fx-viewer");
    fx.click("rbac-back");
    fx.vcx
        .update(|_, cx| assert_eq!(view.read(cx).page(), Page::Users));
}
