//! `node::Cordon` and `node::Uncordon` through the guard: tier, patch, dry run, read-only, audit.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{ErrorKind, NodeSummary};
use oxikube_ports::{PatchKind, ResourceReader};
use oxikube_testkit::{ResourceCall, node};
use serde_json::{Value, json};

use crate::command_bus::{DispatchError, Outcome};
use crate::guard::Confirmation;
use crate::testing::{Harness, ctx, id};

fn target(name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(id("a"), Gvk::new("", "v1", "Node"), name)
}

fn cordon(name: &str) -> Command {
    Command::NodeCordon {
        target: target(name),
    }
}

fn uncordon(name: &str) -> Command {
    Command::NodeUncordon {
        target: target(name),
    }
}

/// A connected harness whose cluster `a` holds the nodes `n1` and `n2` (never a real node).
fn harness() -> Harness {
    let h = Harness::with_node_handlers();
    h.connect("a", false);
    let resources = h.resources("a");
    resources.insert(node().name("n1").build());
    resources.insert(node().name("n2").cordoned().build());
    h
}

/// `(dry_run, merge body)` of every patch the cluster received.
fn patches(h: &Harness) -> Vec<(bool, Value)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => {
                assert_eq!(patch.kind, PatchKind::Merge);
                (options.dry_run, patch.body)
            }
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

fn stored(h: &Harness, name: &str) -> NodeSummary {
    let node = h
        .resources("a")
        .get(&Gvk::new("", "v1", "Node"), None, name)
        .now_or_never()
        .unwrap()
        .unwrap();
    NodeSummary::from_resource(&node).unwrap()
}

use futures::FutureExt as _;

#[test]
fn cordon_asks_one_click_then_dry_runs_and_patches_unschedulable() {
    let h = harness();
    let request = h.ask(cordon("n1"), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Low));
    assert_eq!(request.expected_name, None);
    assert!(request.summary.contains("n1"), "{}", request.summary);
    assert!(
        h.writes("a").is_empty(),
        "nothing is sent before the answer"
    );

    let out = h
        .dispatch(
            cordon("n1"),
            ctx(Initiator::Ui).with_confirmation(Confirmation::simple(request.token)),
        )
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(output.message.as_deref(), Some("Cordoned node n1"));
    let body = json!({"spec": {"unschedulable": true}});
    assert_eq!(patches(&h), [(true, body.clone()), (false, body)]);
    assert!(!stored(&h, "n1").schedulable);
}

#[test]
fn uncordon_removes_the_field_like_kubectl() {
    let h = harness();
    h.confirm_and_run(uncordon("n2"), ctx(Initiator::Ui))
        .unwrap();
    let body = json!({"spec": {"unschedulable": null}});
    assert_eq!(patches(&h), [(true, body.clone()), (false, body)]);
    let after = stored(&h, "n2");
    assert!(after.schedulable);
    assert_eq!(&*after.status, "Ready");
}

#[test]
fn a_dispatch_that_is_a_dry_run_stops_after_the_server_dry_run() {
    let h = harness();
    let out = h
        .confirm_and_run(cordon("n1"), ctx(Initiator::Ui).with_dry_run(true))
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: node n1 can be cordoned")
    );
    assert_eq!(patches(&h).len(), 1);
    assert!(patches(&h)[0].0, "the one request is a dry run");
    assert!(stored(&h, "n1").schedulable, "nothing changed");
    assert!(h.audit()[0].dry_run);
}

#[test]
fn read_only_mode_blocks_both_before_any_request_and_audits_the_denial() {
    let h = Harness::with_node_handlers();
    h.connect("a", true);
    h.resources("a").insert(node().name("n1").build());
    for command in [cordon("n1"), uncordon("n1")] {
        let err = h.dispatch(command, ctx(Initiator::Ui)).unwrap_err();
        assert_eq!(err.read_only_cluster(), Some(&id("a")));
    }
    assert!(h.writes("a").is_empty(), "no request left");
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Denied, Initiator::Ui),
            (AuditOutcome::Denied, Initiator::Ui)
        ]
    );
}

#[test]
fn the_audit_record_names_the_node_the_command_and_the_initiator() {
    let h = harness();
    h.confirm_and_run(cordon("n1"), ctx(Initiator::Ui)).unwrap();
    h.confirm_and_run(uncordon("n2"), ctx(Initiator::Agent))
        .unwrap();
    let audit = h.audit();
    assert_eq!(audit.len(), 2);
    assert_eq!(&*audit[0].cmd, "node::Cordon");
    assert_eq!(audit[0].target, target("n1"));
    assert_eq!(
        (audit[0].initiator, audit[0].outcome),
        (Initiator::Ui, AuditOutcome::Succeeded)
    );
    assert_eq!(&*audit[1].cmd, "node::Uncordon");
    assert_eq!(audit[1].target, target("n2"));
    assert_eq!(audit[1].initiator, Initiator::Agent);
    // No secret or object body rides along.
    assert!(audit.iter().all(|r| r.detail.is_none()));
}

#[test]
fn an_unknown_node_fails_at_the_dry_run_and_changes_nothing() {
    let h = harness();
    let err = h
        .confirm_and_run(cordon("ghost"), ctx(Initiator::Ui))
        .unwrap_err();
    let DispatchError::Handler(error) = err else {
        panic!("expected the handler's error, got {err:?}");
    };
    assert_ne!(error.kind(), ErrorKind::Forbidden);
    assert_eq!(patches(&h).len(), 1, "only the rehearsal was tried");
    assert_eq!(h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}

#[test]
fn a_target_that_is_not_a_node_is_refused_before_any_request() {
    let h = harness();
    let deployment = Gvk::new("apps", "v1", "Deployment");
    let not_nodes = [
        ResourceRef::cluster_scoped(id("a"), deployment.clone(), "web"),
        ResourceRef::namespaced(id("a"), deployment, "x", "web"),
        // A node name under a namespace is not a node either.
        ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "Node"), "x", "n1"),
    ];
    for target in not_nodes {
        for command in [
            Command::NodeCordon {
                target: target.clone(),
            },
            Command::NodeUncordon {
                target: target.clone(),
            },
        ] {
            let err = h
                .confirm_and_run(command, ctx(Initiator::Agent))
                .unwrap_err();
            let DispatchError::Handler(error) = err else {
                panic!("expected the handler's error, got {err:?}");
            };
            assert_eq!(error.kind(), ErrorKind::Validation, "{target:?}");
        }
    }
    assert!(h.writes("a").is_empty(), "no request left");
    assert!(
        h.outcomes()
            .iter()
            .all(|(outcome, _)| *outcome == AuditOutcome::Failed),
        "each refusal is audited as a failure, never a success"
    );
}

#[test]
fn both_commands_are_mutating_tools_the_agent_phase_can_call() {
    let h = harness();
    for (id, name) in [
        (CommandId::NODE_CORDON, "k8s.node_cordon"),
        (CommandId::NODE_UNCORDON, "k8s.node_uncordon"),
    ] {
        let tool = h.bus.tool(id).expect("a tool stub");
        assert_eq!(tool.name.as_str(), name);
        assert!(tool.is_mutating());
        assert!(!tool.destructive_hint(), "a reversible change");
    }
}
