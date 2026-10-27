//! The can-i dialog (E17-S08) in a workspace over the real `CanIService` and the testkit's fake
//! cluster: prefill from a selection, the three answers with their reasons, an impersonated
//! subject, the copyable result and the session-only memory.

use std::sync::Arc;

use futures::{FutureExt as _, StreamExt as _};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_app::ClusterSessionManager;
use oxikube_app::command_bus::{
    CommandBus, CommandRegistry, CommandTarget, DispatchContext, Outcome, commands_for,
};
use oxikube_app::guard::MutationGuard;
use oxikube_app::search::aliases::AliasRegistry;
use oxikube_app::session::SessionOptions;
use oxikube_app::tools::rbac::{CanIMethod, CanIRequest, CanIService, CanISubject};
use oxikube_domain::access::{AccessCheck, AccessDecision, Impersonate};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, Gvr, ResourceRef};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_rbac_ui::can_i::{
    CanIDialog, CanIParts, CanIRequestSink, CheckState, SessionMemory, SubjectKind,
    register_commands,
};
use oxikube_testkit::fakes::{
    FakeAccessReviewPort, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort,
    FakeStatePort,
};
use oxikube_workspace::Workspace;
use oxikube_workspace::test_support::open_workspace;

fn context() -> ClusterContext {
    let name = ContextName::new("kind-oxikube");
    ClusterContext::new(
        ClusterId::new("/kubeconfig", &name),
        name,
        SourceId("kubeconfig".into()),
    )
}

fn cluster() -> ClusterId {
    context().cluster
}

struct Fixture {
    workspace: Entity<Workspace>,
    vcx: VisualTestContext,
    service: CanIService,
    access: Arc<FakeAccessReviewPort>,
    memory: SessionMemory,
}

fn setup(cx: &mut TestAppContext) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context()]));
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
    sessions.open(&context(), SessionOptions::default());
    sessions
        .connect(&cluster())
        .now_or_never()
        .expect("the fakes answer at once")
        .expect("connected");
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    Fixture {
        workspace,
        vcx,
        service: CanIService::new(sessions, AliasRegistry::new(), clock),
        access: connector.ports_for(&cluster()).access,
        memory: SessionMemory::new(),
    }
}

impl Fixture {
    fn open(&mut self, prefill: CanIRequest) -> Entity<CanIDialog> {
        let parts = CanIParts {
            cluster: cluster(),
            service: self.service.clone(),
            prefill,
            memory: self.memory.clone(),
        };
        let workspace = self.workspace.clone();
        self.vcx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                let dialog = cx.new(|cx| CanIDialog::new(parts, window, cx));
                workspace.show_modal(dialog.clone(), window, cx);
                dialog
            })
        })
    }

    fn ask(&mut self, dialog: &Entity<CanIDialog>) {
        dialog.update_in(&mut self.vcx, |dialog, window, cx| dialog.ask(window, cx));
        self.vcx.run_until_parked();
    }

    fn state(&mut self, dialog: &Entity<CanIDialog>) -> CheckState {
        self.vcx.read(|cx| dialog.read(cx).state().clone())
    }

    fn set(&mut self, dialog: &Entity<CanIDialog>, field: &str, value: &str) {
        dialog.update_in(&mut self.vcx, |dialog, window, cx| {
            let input = match field {
                "verb" => &dialog.verb_input(),
                "resource" => &dialog.resource_input(),
                "namespace" => &dialog.namespace_input(),
                _ => &dialog.subject_input(),
            };
            input.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
        });
    }
}

fn pods(verb: &str) -> AccessCheck {
    AccessCheck::new(verb, Gvr::new("", "v1", "pods")).namespace("shop")
}

fn blank() -> CanIRequest {
    CanIRequest::default()
}

#[gpui::test]
fn the_dialog_is_prefilled_from_the_selected_object(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0");
    let prefill = f.service.prefill(&cluster(), &pod, Some("delete"));
    let dialog = f.open(prefill);
    let request = f
        .vcx
        .read(|cx| dialog.read(cx).request(cx))
        .expect("request");
    assert_eq!(request.verb, "delete");
    assert_eq!(request.resource, "pods");
    assert_eq!(request.namespace.as_deref(), Some("shop"));
    assert_eq!(request.name.as_deref(), Some("web-0"));
    assert_eq!(request.subject, CanISubject::Myself);
}

#[gpui::test]
fn denied_shows_the_reason_and_the_review_that_gave_it(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.access.set_decision(
        pods("delete"),
        AccessDecision::Denied {
            reason: Some("RBAC: no rule matched".into()),
        },
    );
    let dialog = f.open(CanIRequest {
        verb: "delete".into(),
        resource: "pods".into(),
        namespace: Some("shop".into()),
        ..blank()
    });
    f.ask(&dialog);
    let CheckState::Answered(answer) = f.state(&dialog) else {
        panic!("answered");
    };
    assert_eq!(answer.outcome(), "Denied");
    assert_eq!(answer.reason(), "RBAC: no rule matched");
    assert_eq!(answer.method, CanIMethod::SelfSubjectAccessReview);
    f.vcx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    for selector in ["can-i-outcome", "can-i-reason", "can-i-method"] {
        assert!(
            f.vcx.debug_bounds(selector).is_some(),
            "{selector} is drawn"
        );
    }
}

#[gpui::test]
fn allowed_and_unknown_are_distinct_answers(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.access.set_decision(
        pods("patch"),
        AccessDecision::Unknown {
            reason: "webhook down".into(),
        },
    );
    let dialog = f.open(CanIRequest {
        verb: "get".into(),
        resource: "pods".into(),
        namespace: Some("shop".into()),
        ..blank()
    });
    f.ask(&dialog);
    assert!(matches!(f.state(&dialog), CheckState::Answered(a) if a.outcome() == "Allowed"));
    f.set(&dialog, "verb", "patch");
    f.ask(&dialog);
    assert!(
        matches!(f.state(&dialog), CheckState::Answered(a) if a.outcome() == "Unknown" && a.reason() == "webhook down")
    );
}

#[gpui::test]
fn an_impersonated_serviceaccount_is_a_subject_review(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    f.access.set_decision(
        pods("delete").impersonating(Impersonate::service_account("shop", "bot")),
        AccessDecision::Denied { reason: None },
    );
    let dialog = f.open(CanIRequest {
        verb: "delete".into(),
        resource: "pods".into(),
        namespace: Some("shop".into()),
        ..blank()
    });
    dialog.update_in(&mut f.vcx, |dialog, _, cx| {
        dialog.set_subject_kind(SubjectKind::ServiceAccount, cx);
    });
    f.set(&dialog, "subject", "shop/bot");
    f.ask(&dialog);
    let CheckState::Answered(answer) = f.state(&dialog) else {
        panic!("answered");
    };
    assert_eq!(answer.outcome(), "Denied");
    assert_eq!(answer.method, CanIMethod::SubjectAccessReview);
}

#[gpui::test]
fn a_bad_subject_or_empty_fields_fail_without_asking(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let dialog = f.open(blank());
    f.ask(&dialog);
    assert!(
        matches!(f.state(&dialog), CheckState::Failed(_)),
        "no resource"
    );
    dialog.update_in(&mut f.vcx, |dialog, _, cx| {
        dialog.set_subject_kind(SubjectKind::ServiceAccount, cx);
    });
    f.set(&dialog, "resource", "pods");
    f.set(&dialog, "subject", "no-namespace");
    f.ask(&dialog);
    assert!(matches!(f.state(&dialog), CheckState::Failed(_)));
    assert_eq!(f.access.can_i_calls(), 0);
}

#[gpui::test]
fn the_result_is_copyable_text(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let dialog = f.open(CanIRequest {
        verb: "get".into(),
        resource: "pods".into(),
        namespace: Some("shop".into()),
        ..blank()
    });
    assert!(
        !dialog.update(&mut f.vcx, |dialog, cx| dialog.copy_result(cx)),
        "nothing to copy yet"
    );
    f.ask(&dialog);
    assert!(dialog.update(&mut f.vcx, |dialog, cx| dialog.copy_result(cx)));
    let copied = f
        .vcx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("text");
    assert!(
        copied.contains("get pods in namespace shop: Allowed"),
        "{copied}"
    );
    assert!(copied.contains("answered by: SelfSubjectAccessReview"));
    assert!(copied.contains("kubectl auth can-i get pods -n shop"));
}

#[gpui::test]
fn the_last_verb_and_subject_are_remembered_for_the_session_only(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let dialog = f.open(CanIRequest {
        verb: "list".into(),
        resource: "pods".into(),
        ..blank()
    });
    dialog.update_in(&mut f.vcx, |dialog, _, cx| {
        dialog.set_subject_kind(SubjectKind::User, cx);
    });
    f.set(&dialog, "subject", "jane");
    f.ask(&dialog);
    dialog.update(&mut f.vcx, |dialog, cx| dialog.cancel(cx));
    let reopened = f.open(blank());
    let request = f
        .vcx
        .read(|cx| reopened.read(cx).request(cx))
        .expect("request");
    assert_eq!(request.verb, "list");
    assert_eq!(request.subject, CanISubject::User("jane".into()));
    assert_eq!(f.memory.subject(), (SubjectKind::User, "jane".to_owned()));
}

#[gpui::test]
fn a_new_ask_replaces_the_review_in_flight(cx: &mut TestAppContext) {
    let mut f = setup(cx);
    let dialog = f.open(CanIRequest {
        verb: "get".into(),
        resource: "pods".into(),
        ..blank()
    });
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.ask(window, cx);
        assert_eq!(*dialog.state(), CheckState::Checking);
        dialog.ask(window, cx);
    });
    f.vcx.run_until_parked();
    assert!(matches!(f.state(&dialog), CheckState::Answered(_)));
}

fn bus(sink: CanIRequestSink) -> CommandBus {
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, sink).expect("registers once");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    CommandBus::new(
        registry,
        MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock),
    )
}

#[test]
fn the_command_is_read_only_has_a_tool_stub_and_queues_the_dialog() {
    let (sink, mut rx) = CanIRequestSink::channel();
    let bus = bus(sink);
    let command = Command::RbacCanI {
        cluster: cluster(),
        target: None,
        verb: Some("get".into()),
    };
    assert!(!command.is_mutating());
    assert_eq!(command.id(), CommandId::RBAC_CAN_I);
    assert_eq!(
        command::lookup(CommandId::RBAC_CAN_I)
            .expect("declared")
            .title,
        "RBAC: Can I?"
    );
    assert_eq!(
        bus.tool(CommandId::RBAC_CAN_I)
            .expect("a tool stub")
            .name
            .as_str(),
        "k8s.rbac_can_i"
    );
    let outcome = futures::executor::block_on(
        bus.dispatch(command, DispatchContext::new(Initiator::Command, "palette")),
    )
    .expect("dispatched");
    assert!(matches!(outcome, Outcome::Completed(_)));
    let open = futures::executor::block_on(rx.next()).expect("queued");
    assert_eq!(
        (open.cluster, open.verb.as_deref()),
        (cluster(), Some("get"))
    );
}

#[test]
fn the_palette_builds_the_command_from_the_selection() {
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0");
    let target = CommandTarget::none()
        .in_cluster(cluster())
        .selecting(vec![pod.clone()]);
    let commands = commands_for(CommandId::RBAC_CAN_I, &target).expect("built");
    assert_eq!(
        commands,
        [Command::RbacCanI {
            cluster: cluster(),
            target: Some(pod),
            verb: None
        }]
    );
    let none = commands_for(
        CommandId::RBAC_CAN_I,
        &CommandTarget::none().in_cluster(cluster()),
    )
    .expect("built");
    assert!(matches!(none[0], Command::RbacCanI { target: None, .. }));
}
