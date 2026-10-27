//! The panel in a test window over fixed data and a recording command bus.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Modifiers, TestAppContext};
use oxikube_app::access::PolicyStatus;
use oxikube_app::store::{ResourceStore, StoreOptions, StorePorts, StoreRuntime};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::rbac::{RbacKind, Subject};
use oxikube_resources_ui::kinds::KindSpecs;
use oxikube_testkit::gpui_test::{TestApp, TestWindow};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort};
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

/// A store over a fake port, its feeds run on threads of their own (they end when aborted).
fn store_over(port: &Arc<FakeResourcePort>, cluster: &str) -> ResourceStore {
    let clock = Arc::new(FakeClockPort::default());
    ResourceStore::new(
        oxikube_domain::ids::ClusterId::new(
            "live.yaml",
            &oxikube_domain::ids::ContextName::new(cluster),
        ),
        StorePorts {
            resources: port.clone(),
            tables: Arc::new(FakeTableFeedPort::with_clock(clock.clone())),
        },
        StoreRuntime {
            spawner: Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
                std::thread::spawn(move || futures::executor::block_on(task));
            }),
            clock,
            probe: None,
        },
        StoreOptions::default(),
    )
}

fn ci_binding(name: &str) -> oxikube_domain::Resource {
    binding(
        Some("shop"),
        name,
        "viewer",
        json!([{"kind": "ServiceAccount", "name": "ci", "namespace": "shop"}]),
    )
}

/// Lets the panel's poll loop and the feeds run until `done` holds (or fails after five seconds).
fn until(
    cx: &mut TestAppContext,
    window: &mut TestWindow<ServiceAccountPanel>,
    what: &str,
    mut done: impl FnMut(&mut TestWindow<ServiceAccountPanel>) -> bool,
) {
    for _ in 0..500 {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        window.draw_frame();
        if done(window) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

#[gpui::test]
fn the_live_panel_follows_the_feed_and_moves_to_the_store_of_a_reconnect(cx: &mut TestAppContext) {
    cx.update(oxikube_ui::init);
    let port = Arc::new(FakeResourcePort::default());
    port.insert(cluster_role("viewer"));
    port.insert(ci_binding("ci-views"));
    let first = store_over(&port, "kind-live-a");
    let cell = Rc::new(RefCell::new(None));
    let facts = ServiceAccountFacts::from_resource(&service_account(json!({})));
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(Bus::default());
    let mut app = TestApp::new(cx);
    let shared = cell.clone();
    let mut window = app.open_window(move |_, cx| {
        ServiceAccountPanel::following(target(), facts, shared, dispatcher, cx)
    });
    window.draw_frame();

    // Not connected: nothing is read, and the panel says it is waiting.
    assert!(window.bounds("sa-bound-loading").is_some());

    *cell.borrow_mut() = Some(first);
    until(cx, &mut window, "the bound role to appear", |w| {
        w.bounds("sa-bound-row-0").is_some()
    });
    assert!(window.bounds("sa-bound-loading").is_none());
    assert!(window.bounds("sa-bound-row-1").is_none());

    // A reconnect brings a new store with one more binding: the panel moves to it.
    let more = Arc::new(FakeResourcePort::default());
    more.insert(cluster_role("viewer"));
    more.insert(ci_binding("ci-views"));
    more.insert(ci_binding("ci-more"));
    *cell.borrow_mut() = Some(store_over(&more, "kind-live-b"));
    until(cx, &mut window, "the second binding to appear", |w| {
        w.bounds("sa-bound-row-1").is_some()
    });

    // Another reconnect, to a cluster with nothing bound: the old answer is dropped.
    let empty = Arc::new(FakeResourcePort::default());
    *cell.borrow_mut() = Some(store_over(&empty, "kind-live-c"));
    until(cx, &mut window, "the new store's answer", |w| {
        w.bounds("sa-bound-row-0").is_none() && w.bounds("sa-bound-none").is_some()
    });
}
