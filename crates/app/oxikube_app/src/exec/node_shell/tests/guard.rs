//! `node::Shell` through the guard: read-only, the confirmation, the dry run and the audit of the
//! create.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{ClusterPrefs, NODE_SHELL_LABEL};
use oxikube_testkit::ResourceCall;

use super::{Fixture, custom_prefs};
use crate::command_bus::{DispatchError, Outcome};
use crate::testing::{ctx, pod};

#[test]
fn a_read_only_cluster_refuses_a_node_shell_for_every_initiator() {
    let f = Fixture::with(ClusterPrefs {
        read_only: true,
        // The pod shell setting does not lift it: a node shell is a mutation, not an exec.
        exec_in_read_only: true,
        ..ClusterPrefs::default()
    });
    for initiator in Initiator::ALL {
        let err = f.h.dispatch(f.command(), ctx(initiator)).unwrap_err();
        assert!(
            matches!(err, DispatchError::ReadOnly { .. }),
            "{initiator}: {err:?}"
        );
        assert_eq!(OxiError::from(err).kind(), ErrorKind::Forbidden);
    }
    assert!(
        f.resources.mutating_calls().is_empty(),
        "no pod was dry-run"
    );
    assert!(f.exec.recorded_calls().is_empty(), "no pod was created");
    assert!(f.opened.lock().is_empty(), "no terminal was asked for");
    assert_eq!(f.h.bus.guard().pending_confirmations(), 0, "it never asks");
    let audit = f.h.audit();
    assert_eq!(audit.len(), Initiator::ALL.len());
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn the_confirmation_is_medium_and_names_the_node_and_the_image() {
    let f = Fixture::new();
    let request = f.h.ask(f.command(), ctx(Initiator::Ui));
    assert_eq!(request.command, CommandId::NODE_SHELL);
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Medium));
    assert!(request.summary.contains("worker-1"), "{}", request.summary);
    assert!(
        request.summary.contains("busybox:1.37"),
        "{}",
        request.summary
    );
    assert!(
        request.summary.contains("kube-system"),
        "{}",
        request.summary
    );
    assert!(
        request.summary.contains("privileged"),
        "{}",
        request.summary
    );
    assert!(
        f.resources.mutating_calls().is_empty(),
        "nothing before the answer"
    );
    assert!(f.exec.recorded_calls().is_empty());
}

#[test]
fn the_confirmation_names_the_image_the_settings_chose() {
    let f = Fixture::with(custom_prefs());
    let request = f.h.ask(f.command(), ctx(Initiator::Ui));
    assert!(
        request.summary.contains("registry.local/tools:2"),
        "{}",
        request.summary
    );
    assert!(request.summary.contains("ops-debug"), "{}", request.summary);
}

#[test]
fn declining_the_confirmation_creates_nothing_and_is_audited() {
    let f = Fixture::new();
    let request = f.h.ask(f.command(), ctx(Initiator::Ui));
    futures::FutureExt::now_or_never(f.h.bus.decline(request.token))
        .expect("no wait")
        .expect("declined");
    assert!(f.resources.mutating_calls().is_empty());
    assert!(f.exec.recorded_calls().is_empty());
    assert_eq!(f.h.outcomes(), [(AuditOutcome::Cancelled, Initiator::Ui)]);
}

#[test]
fn a_confirmed_shell_dry_runs_the_pod_through_the_guard_and_queues_the_terminal() {
    let f = Fixture::with(custom_prefs());
    let outcome = f.run().expect("runs");
    assert!(matches!(outcome, Outcome::Completed(_)), "{outcome:?}");

    let writes = f.resources.mutating_calls();
    assert_eq!(writes.len(), 1, "{writes:?}");
    let ResourceCall::Create {
        kind,
        namespace,
        object,
        options,
    } = &writes[0]
    else {
        panic!("expected a create, got {writes:?}");
    };
    assert_eq!(&*kind.kind, "Pod");
    assert_eq!(namespace.as_deref(), Some("ops-debug"));
    assert!(
        options.dry_run,
        "the guard only validates; the adapter creates"
    );
    assert_eq!(object["spec"]["nodeName"], "worker-1");
    assert_eq!(
        object["spec"]["containers"][0]["image"],
        "registry.local/tools:2"
    );
    assert_eq!(object["spec"]["activeDeadlineSeconds"], 900);
    assert_eq!(object["metadata"]["labels"]["team"], "infra");
    assert_eq!(object["metadata"]["labels"][NODE_SHELL_LABEL], "true");

    assert_eq!(*f.opened.lock(), [f.node()], "the terminal was asked for");
    assert!(
        f.exec.recorded_calls().is_empty(),
        "no session before the terminal opens"
    );
}

#[test]
fn the_audit_names_the_node_the_image_the_namespace_and_the_initiator() {
    let f = Fixture::with(custom_prefs());
    f.run().expect("runs");
    let audit = f.h.audit();
    assert_eq!(audit.len(), 1);
    let record = &audit[0];
    assert_eq!(&*record.cmd, "node::Shell");
    assert_eq!(record.initiator, Initiator::Ui);
    assert_eq!(&*record.who, "alice");
    assert_eq!(&*record.target.name, "worker-1");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        record.detail.as_deref(),
        Some("phase=create image=registry.local/tools:2 namespace=ops-debug")
    );
}

#[test]
fn a_dry_run_dispatch_validates_the_pod_and_opens_nothing() {
    let f = Fixture::new();
    f.run_with(ctx(Initiator::Ui).with_dry_run(true))
        .expect("runs");
    assert_eq!(f.resources.mutating_calls().len(), 1);
    assert!(f.opened.lock().is_empty(), "no terminal for a dry run");
    let audit = f.h.audit();
    assert!(audit[0].dry_run);
    // And no permit was left behind for a later open.
    let open = futures::FutureExt::now_or_never(f.service.open_node_shell(&f.node()));
    assert_eq!(open.unwrap().err().unwrap().kind(), ErrorKind::Forbidden);
}

#[test]
fn an_admission_refusal_of_the_dry_run_says_what_to_change_and_opens_nothing() {
    let f = Fixture::new();
    f.resources.script().create.push_err(OxiError::forbidden(
        "pods \"x\" is forbidden: violates PodSecurity \"baseline:latest\": privileged",
    ));
    let err = f.run().unwrap_err();
    let DispatchError::Handler(err) = err else {
        panic!("expected a handler error, got {err:?}");
    };
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("pod security admission"), "{err}");
    assert!(err.message().contains("node_shell.namespace"), "{err}");
    assert!(err.message().contains("kube-system"), "{err}");
    assert!(f.opened.lock().is_empty());
    assert!(f.exec.recorded_calls().is_empty(), "no pod was created");
    assert_eq!(f.h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}

#[test]
fn rbac_and_quota_refusals_are_told_apart() {
    for (message, wanted) in [
        (
            "pods is forbidden: User cannot create resource \"pods\"",
            "not allowed to create pods",
        ),
        (
            "pods is forbidden: exceeded quota: compute",
            "resource quota",
        ),
    ] {
        let f = Fixture::new();
        f.resources
            .script()
            .create
            .push_err(OxiError::forbidden(message));
        let DispatchError::Handler(err) = f.run().unwrap_err() else {
            panic!("expected a handler error");
        };
        assert!(err.message().contains(wanted), "{err}");
    }
}

#[test]
fn only_a_node_can_be_shelled_into() {
    let f = Fixture::new();
    let command = Command::NodeShell {
        target: pod("a", "web-0"),
    };
    let err =
        f.h.confirm_and_run(command, ctx(Initiator::Ui))
            .unwrap_err();
    let DispatchError::Handler(err) = err else {
        panic!("expected a handler error, got {err:?}");
    };
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(f.resources.mutating_calls().is_empty());
}

#[test]
fn a_blank_image_fails_before_the_cluster_is_asked() {
    let f = Fixture::with(ClusterPrefs {
        node_shell_image: Some("busybox".into()),
        node_shell: oxikube_ports::NodeShellPrefs {
            image_pull_policy: Some("Sometimes".into()),
            ..Default::default()
        },
        ..ClusterPrefs::default()
    });
    let DispatchError::Handler(err) = f.run().unwrap_err() else {
        panic!("expected a handler error");
    };
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(f.resources.mutating_calls().is_empty());
}

#[test]
fn the_tool_stub_is_unsafe_interactive_and_hidden_from_agents() {
    let f = Fixture::new();
    let tool = f.h.bus.tool(CommandId::NODE_SHELL).expect("a tool stub");
    assert_eq!(tool.name.as_str(), "k8s.node_shell");
    assert!(tool.annotations.unsafe_ && tool.annotations.interactive);
    assert!(tool.annotations.agent_hidden && !tool.agent_exposed_by_default());
    assert_eq!(tool.risk, Some(Risk::Medium));
    assert!(
        f.h.bus
            .agent_tools(false)
            .all(|t| t.name.as_str() != "k8s.node_shell")
    );
    assert!(
        f.h.bus
            .agent_tools(true)
            .any(|t| t.name.as_str() == "k8s.node_shell")
    );
}
