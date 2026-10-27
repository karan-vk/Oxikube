//! The role and subjects in a binding's detail.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::kinds::core_kinds;

use super::rig::{Rig, cluster, cluster_binding, cluster_role, role_binding, target};
use crate::binding::{BindingPanel, RolePhase};

fn panel_of(
    rig: &mut Rig,
    detail: &gpui::Entity<oxikube_resources_ui::detail::DetailView>,
) -> gpui::Entity<BindingPanel> {
    rig.vcx.update(|_, cx| {
        detail
            .read(cx)
            .kind_panel()
            .expect("the kind has a panel")
            .clone()
            .downcast::<BindingPanel>()
            .expect("the binding panel")
    })
}

#[gpui::test]
fn the_detail_lists_the_subjects_and_the_rules_the_role_grants(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(
        core_kinds(),
        vec![role_binding("view", "3"), cluster_role("view")],
    );
    let detail = rig.open_detail(&target("RoleBinding", Some("demo"), "readers"));
    let panel = panel_of(&mut rig, &detail);
    let (subjects, rules, phase) = rig.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        (
            panel.subjects().to_vec(),
            panel.rules().to_vec(),
            panel.phase().clone(),
        )
    });
    assert_eq!(phase, RolePhase::Ready);
    assert_eq!(subjects.len(), 2);
    assert_eq!((subjects[0].kind, &*subjects[0].name), ("User", "ana"));
    assert_eq!(subjects[0].api_group, "rbac.authorization.k8s.io");
    assert_eq!(
        (
            subjects[1].kind,
            &*subjects[1].namespace,
            subjects[1].api_group
        ),
        ("ServiceAccount", "demo", "")
    );
    assert_eq!(rules.len(), 2);
    for selector in [
        "binding-panel",
        "binding-kind",
        "binding-role-link",
        "binding-subject-count",
        "binding-subjects-header",
        "binding-subject-row-0",
        "binding-subject-row-1",
        "binding-rules-title",
        "binding-rule-0",
    ] {
        assert!(rig.is_laid_out(selector), "{selector}");
    }
}

#[gpui::test]
fn a_role_that_does_not_exist_is_said_and_the_binding_grants_nothing(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(core_kinds(), vec![role_binding("gone", "3")]);
    let detail = rig.open_detail(&target("RoleBinding", Some("demo"), "readers"));
    let panel = panel_of(&mut rig, &detail);
    assert_eq!(
        rig.vcx.update(|_, cx| panel.read(cx).phase().clone()),
        RolePhase::Missing
    );
    assert!(rig.is_laid_out("binding-rules-missing"));
}

#[gpui::test]
fn a_long_subject_list_is_virtualised(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(
        core_kinds(),
        vec![cluster_binding("view", 500), cluster_role("view")],
    );
    let detail = rig.open_detail(&target("ClusterRoleBinding", None, "admins"));
    assert!(rig.is_laid_out("binding-subject-row-0"));
    assert!(
        !rig.is_laid_out("binding-subject-row-400"),
        "only the rows on screen are drawn"
    );
    let panel = panel_of(&mut rig, &detail);
    assert_eq!(rig.vcx.update(|_, cx| panel.read(cx).subjects().len()), 501);
}

#[gpui::test]
fn the_role_name_opens_the_role_and_edit_and_delete_send_their_commands(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(
        core_kinds(),
        vec![role_binding("view", "3"), cluster_role("view")],
    );
    let this = target("RoleBinding", Some("demo"), "readers");
    rig.open_detail(&this);
    rig.click("binding-role-link");
    rig.click("binding-edit");
    rig.click("binding-delete");
    assert_eq!(
        rig.sent.sent(),
        [
            Command::ResourceOpen {
                target: target("ClusterRole", None, "view")
            },
            Command::BindingOpenDialog {
                cluster: Some(cluster()),
                cluster_binding: false,
                target: Some(this.clone()),
            },
            Command::BindingDelete { target: this },
        ]
    );
}

#[gpui::test]
fn a_read_only_cluster_disables_the_buttons(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(core_kinds(), vec![role_binding("view", "3")]);
    rig.sessions.set_read_only(&cluster(), true).unwrap();
    rig.open_detail(&target("RoleBinding", Some("demo"), "readers"));
    rig.click("binding-delete");
    assert!(
        rig.sent.sent().is_empty(),
        "a disabled button sends nothing"
    );
}

#[gpui::test]
fn the_kinds_are_registered_with_the_panel_and_the_dialog_among_their_actions(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    rig.vcx.update(|_, cx| {
        for (kind, namespaced) in [("RoleBinding", true), ("ClusterRoleBinding", false)] {
            let gvk = Gvk::new("rbac.authorization.k8s.io", "v1", kind);
            let spec = oxikube_resources_ui::kinds::KindSpecs::kind_for(cx, &gvk);
            assert!(spec.offers(oxikube_domain::command::CommandId::BINDING_OPEN_DIALOG));
            assert!(spec.offers(oxikube_domain::command::CommandId::BINDING_DELETE));
            assert_eq!(
                spec.scope == oxikube_domain::ids::Scope::Namespaced,
                namespaced
            );
            let detail = oxikube_resources_ui::kinds::KindSpecs::detail_for(cx, &gvk);
            assert!(!detail.is_generic(), "{kind} has a panel");
        }
    });
}
