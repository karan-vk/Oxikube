//! The panel in a test window over fixed data and a recording command bus.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Modifiers, TestAppContext};
use oxikube_app::access::PolicyStatus;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::rbac::{RbacKind, Subject};
use oxikube_resources_ui::kinds::KindSpecs;
use oxikube_testkit::gpui_test::{TestApp, TestWindow};
use oxikube_workspace::CommandDispatcher;
use serde_json::{Value, json};

use super::{binding, cluster, cluster_role, index_of, service_account};
use crate::service_account::{
    self, BoundRoles, HeaderAction, ServiceAccountFacts, ServiceAccountPanel,
};

#[derive(Clone, Default)]
struct Bus(Rc<RefCell<Vec<Command>>>);

impl CommandDispatcher for Bus {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

fn target() -> ResourceRef {
    ResourceRef::namespaced(
        cluster(),
        Gvk::new("", "v1", "ServiceAccount"),
        "shop",
        "ci",
    )
}

fn bound(status: PolicyStatus) -> BoundRoles {
    let index = index_of(vec![
        cluster_role("viewer"),
        binding(
            Some("shop"),
            "ci-views",
            "viewer",
            json!([{"kind": "ServiceAccount", "name": "ci", "namespace": "shop"}]),
        ),
        binding(
            None,
            "everyone-views",
            "viewer",
            json!([{"kind": "Group", "name": "system:serviceaccounts", "apiGroup": "rbac.authorization.k8s.io"}]),
        ),
    ]);
    BoundRoles::of_index(&index, status, &Subject::service_account("shop", "ci"))
}

struct Rig {
    window: TestWindow<ServiceAccountPanel>,
    bus: Bus,
}

impl Rig {
    fn new(cx: &mut TestAppContext, fields: Value, bound: BoundRoles) -> Self {
        cx.update(oxikube_ui::init);
        let bus = Bus::default();
        let facts = ServiceAccountFacts::from_resource(&service_account(fields));
        let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(bus.clone());
        let mut app = TestApp::new(cx);
        let mut window = app.open_window(move |_, _| {
            ServiceAccountPanel::fixed(target(), facts, bound, dispatcher)
        });
        window.draw_frame();
        Self { window, bus }
    }

    fn shown(&mut self, selector: &'static str) -> bool {
        self.window.bounds(selector).is_some()
    }

    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .window
            .bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        self.window
            .simulate_click(bounds.center(), Modifiers::default());
        self.window.draw_frame();
    }

    fn sent(&self) -> Vec<Command> {
        self.bus.0.borrow().clone()
    }
}

#[gpui::test]
fn the_panel_lists_secrets_pull_secrets_automount_and_bound_roles(cx: &mut TestAppContext) {
    let mut rig = Rig::new(
        cx,
        json!({
            "secrets": [{"name": "ci-token"}],
            "imagePullSecrets": [{"name": "registry"}],
            "automountServiceAccountToken": false,
        }),
        bound(PolicyStatus::Ready),
    );
    for selector in [
        "service-account-panel",
        "sa-secrets-0",
        "sa-pull-secrets-0",
        "sa-automount-value",
        "sa-automount-note",
        "sa-bound-list",
        "sa-bound-row-0",
        "sa-bound-row-1",
        "sa-bound-scope-0",
    ] {
        assert!(rig.shown(selector), "{selector}");
    }
    assert!(!rig.shown("sa-bound-loading"));
    assert!(!rig.shown("sa-bound-none"));
    // The group-reached binding says which group it came through; the direct one does not.
    let groups = (0..2)
        .filter(|ix| {
            let selector: &'static str = Box::leak(format!("sa-bound-group-{ix}").into_boxed_str());
            rig.shown(selector)
        })
        .count();
    assert_eq!(groups, 1);
}

#[gpui::test]
fn loading_empty_and_refused_roles_look_different(cx: &mut TestAppContext) {
    let mut loading = Rig::new(cx, json!({}), BoundRoles::Loading);
    assert!(loading.shown("sa-bound-loading"));
    assert!(!loading.shown("sa-bound-none"));
    assert!(!loading.shown("sa-bound-list"));
    // No secrets: the notes stand where the links would be.
    assert!(!loading.shown("sa-secrets-0"));

    let mut empty = Rig::new(
        cx,
        json!({}),
        BoundRoles::Known {
            roles: Vec::new(),
            missing: Vec::new(),
        },
    );
    assert!(empty.shown("sa-bound-none"));
    assert!(!empty.shown("sa-bound-loading"));

    let mut refused = Rig::new(
        cx,
        json!({}),
        BoundRoles::Known {
            roles: Vec::new(),
            missing: vec![(RbacKind::RoleBinding, "forbidden".to_owned())],
        },
    );
    assert!(refused.shown("sa-bound-missing-RoleBinding"));
}

#[gpui::test]
fn links_and_buttons_send_commands_through_the_bus(cx: &mut TestAppContext) {
    let mut rig = Rig::new(
        cx,
        json!({"secrets": [{"name": "ci-token"}]}),
        bound(PolicyStatus::Ready),
    );
    rig.click("sa-secrets-0");
    rig.click("sa-used-by");
    let sent = rig.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    match &sent[0] {
        Command::ResourceOpen { target } => {
            assert_eq!(&*target.gvk.kind, "Secret");
            assert_eq!(target.namespace(), Some("shop"));
            assert_eq!(&*target.name, "ci-token");
        }
        other => panic!("expected resource::Open of the Secret, got {other:?}"),
    }
    assert_eq!(sent[1].id(), CommandId::RESOURCE_SHOW_USED_BY);
}

#[gpui::test]
fn a_bound_role_opens_its_role_and_its_binding(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, json!({}), bound(PolicyStatus::Ready));
    rig.click("sa-bound-role-0");
    rig.click("sa-bound-binding-0");
    let kinds: Vec<String> = rig
        .sent()
        .into_iter()
        .map(|command| match command {
            Command::ResourceOpen { target } => target.gvk.kind.to_string(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(kinds[0], "ClusterRole");
    assert!(kinds[1].ends_with("Binding"), "{kinds:?}");
}

#[gpui::test]
fn another_story_adds_a_header_action_without_this_module_knowing_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        service_account::register_header_action(
            cx,
            HeaderAction {
                id: "sa-test-action",
                label: "Do it",
                tooltip: "A test action",
                command: |target| Command::ResourceShowUsedBy {
                    target: target.clone(),
                },
            },
        );
    });
    let mut rig = Rig::new(cx, json!({}), bound(PolicyStatus::Ready));
    assert!(rig.shown("sa-action-sa-test-action"));
    rig.click("sa-action-sa-test-action");
    assert_eq!(rig.sent().len(), 1);
}

#[gpui::test]
fn init_gives_the_registered_kind_its_panel(cx: &mut TestAppContext) {
    cx.update(|cx| {
        oxikube_resources_ui::used_by::init(cx);
        let gvk = service_account::gvk();
        assert!(KindSpecs::detail_for(cx, &gvk).is_generic());
        service_account::init(cx);
        assert!(!KindSpecs::detail_for(cx, &gvk).is_generic());
    });
}
