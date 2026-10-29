//! `node::Drain` through the real `CommandBus` and `MutationGuard` over a scripted `DrainPort`:
//! read-only first, the typed name, the flags handed to the algorithm, the per-pod audit records
//! sharing the drain's batch id, the followers, cancel, and the dialog's flow.

use std::sync::Arc;
use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::drain::{
    BlockReason, DrainFlags, DrainProgress, DrainSummary, PodRef, SkipReason, SkippedPod,
};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::ListPage;
use oxikube_testkit::{DrainCall, DrainRun, FakeDrainPort, ResourceCall, node, pod};

use crate::command_bus::{DispatchError, Outcome};
use crate::guard::Confirmation;
use crate::nodes::{DrainFlow, NodeService};
use crate::testing::{Harness, ctx, id, node as node_ref};

fn pod_ref(name: &str) -> PodRef {
    PodRef {
        namespace: "shop".into(),
        name: name.into(),
        uid: Some(format!("{name}-uid")),
    }
}

fn drain(flags: DrainFlags) -> Command {
    Command::NodeDrain {
        target: node_ref("a", "n1"),
        flags,
    }
}

/// A connected harness with node `n1` (never a real node) and a scripted drain port.
fn harness() -> (Harness, Arc<NodeService>, Arc<FakeDrainPort>) {
    let port = Arc::new(FakeDrainPort::new());
    let (h, service) = Harness::with_drain(port.clone());
    h.connect("a", false);
    let resources = h.resources("a");
    resources.insert(node().name("n1").build());
    // The evictions the scripted drains send through the writer are accepted.
    for _ in 0..4 {
        resources.script().evict.push_ok(());
    }
    (h, service, port)
}

/// A drain of `a` and `b`: `a` is blocked by a budget once, then both go; `agent` is skipped.
fn two_pods() -> DrainRun {
    let skipped = vec![SkippedPod {
        pod: pod_ref("agent"),
        reason: SkipReason::DaemonSet,
    }];
    DrainRun::of([
        DrainProgress::Planned {
            evict: vec![pod_ref("a"), pod_ref("b")],
            skipped: skipped.clone(),
        },
        DrainProgress::Cordoned { already: false },
        DrainProgress::Evicting {
            pod: pod_ref("a"),
            attempt: 1,
        },
        DrainProgress::Blocked {
            pod: pod_ref("a"),
            attempt: 1,
            reason: "Cannot evict pod as it would violate the pod's disruption budget.".into(),
            retry_in: Duration::from_secs(1),
        },
        DrainProgress::Evicting {
            pod: pod_ref("a"),
            attempt: 2,
        },
        DrainProgress::Evicted { pod: pod_ref("a") },
        DrainProgress::Gone { pod: pod_ref("a") },
        DrainProgress::Evicting {
            pod: pod_ref("b"),
            attempt: 1,
        },
        DrainProgress::Evicted { pod: pod_ref("b") },
        DrainProgress::Gone { pod: pod_ref("b") },
        DrainProgress::Finished(DrainSummary {
            node: "n1".into(),
            evicted: vec![pod_ref("a"), pod_ref("b")],
            skipped,
            failed: Vec::new(),
            dry_run: false,
        }),
    ])
}

/// `batch=<id>` out of an audit detail.
fn batch_of(record: &AuditRecord) -> String {
    record
        .detail
        .as_deref()
        .and_then(|detail| detail.split(' ').find_map(|w| w.strip_prefix("batch=")))
        .unwrap_or_else(|| panic!("no batch in {record:?}"))
        .to_owned()
}

#[test]
fn read_only_refuses_the_drain_before_anything_is_read_or_written() {
    let (h, _, port) = harness();
    h.manager.set_read_only(&id("a"), true).unwrap();
    let err = h
        .dispatch(drain(DrainFlags::default()), ctx(Initiator::Agent))
        .unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }), "{err:?}");
    assert!(port.recorded_calls().is_empty(), "no drain, no cordon");
    assert!(h.writes("a").is_empty());
    assert_eq!(h.outcomes(), [(AuditOutcome::Denied, Initiator::Agent)]);
}

#[test]
fn the_drain_asks_for_the_node_name_typed() {
    let (h, _, port) = harness();
    let request = h.ask(drain(DrainFlags::default()), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::TypeName);
    assert_eq!(request.risk, Some(Risk::High));
    assert_eq!(request.expected_name.as_deref(), Some("n1"));
    assert!(
        port.recorded_calls().is_empty(),
        "nothing before the answer"
    );
    let wrong = h.dispatch(
        drain(DrainFlags::default()),
        ctx(Initiator::Ui).with_confirmation(Confirmation::typed(request.token, "n2")),
    );
    assert!(wrong.is_err(), "a wrong name is refused");
    assert!(port.recorded_calls().is_empty());
}

#[test]
fn a_drain_runs_the_algorithm_with_the_flags_and_audits_each_pod_in_one_batch() {
    let (h, service, port) = harness();
    port.push(two_pods());
    let mut follower = service.monitor().subscribe(&id("a"), "n1");
    let flags = DrainFlags {
        ignore_daemonsets: true,
        delete_emptydir_data: true,
        force: false,
        grace_period_secs: Some(30),
        timeout_secs: 120,
    };
    let Ok(Outcome::Completed(output)) =
        h.confirm_and_run(drain(flags.clone()), ctx(Initiator::Ui))
    else {
        panic!("the drain completes");
    };
    assert_eq!(
        output.message.as_deref(),
        Some("Drained node n1: 2 pod(s) evicted, 1 skipped")
    );
    let data = output.data.expect("data");
    assert_eq!(data["outcome"], "drained");
    assert_eq!(data["evicted"], serde_json::json!(["shop/a", "shop/b"]));

    // The algorithm got the flags as options; the writes went through the guard's writer.
    let [DrainCall::Drain { node, options }] = &port.recorded_calls()[..] else {
        panic!("one drain");
    };
    assert_eq!(node, "n1");
    assert_eq!(options, &flags.options());
    assert!(!options.dry_run);
    let writes = h.writes("a");
    assert!(
        matches!(writes[0], ResourceCall::Patch { .. }),
        "{writes:?}"
    );
    assert_eq!(
        writes
            .iter()
            .filter(|w| matches!(w, ResourceCall::Evict { .. }))
            .count(),
        2
    );

    // Every step reached the follower, in order.
    let mut seen = Vec::new();
    while let Some(Some(step)) = follower.next().now_or_never() {
        seen.push(step);
    }
    assert_eq!(seen.len(), 11);
    assert!(matches!(seen[3], DrainProgress::Blocked { .. }));
    assert!(matches!(seen.last(), Some(DrainProgress::Finished(_))));
    assert!(!service.monitor().is_running(&id("a"), "n1"));

    // One record per pod, then the drain's own; all share the batch id.
    let audit = h.audit();
    let shown: Vec<_> = audit
        .iter()
        .map(|r| (&*r.cmd, r.target.name.to_string(), r.outcome))
        .collect();
    assert_eq!(
        shown,
        [
            ("node::Drain", "a".to_owned(), AuditOutcome::Succeeded),
            ("node::Drain", "b".to_owned(), AuditOutcome::Succeeded),
            ("node::Drain", "n1".to_owned(), AuditOutcome::Succeeded),
        ]
    );
    let batch = batch_of(&audit[2]);
    assert!(audit.iter().all(|r| batch_of(r) == batch));
    assert!(
        audit
            .iter()
            .all(|r| r.batch_id.as_deref() == Some(batch.as_str())),
        "the batch id is a field of every record too"
    );
    assert_eq!(audit[0].target.namespace(), Some("shop"));
    assert!(audit[0].detail.as_deref().unwrap().contains("node=n1"));
    let drain_detail = audit[2].detail.as_deref().unwrap();
    for words in [
        "ignore_daemonsets=true",
        "delete_emptydir_data=true",
        "grace=30s",
        "timeout=120s",
        "evicted=2 failed=0 skipped=1",
    ] {
        assert!(drain_detail.contains(words), "{words} in {drain_detail}");
    }
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));
}

#[test]
fn pods_left_fail_the_drain_naming_them_with_a_failed_record_each() {
    let (h, _, port) = harness();
    port.push(DrainRun::of([
        DrainProgress::Planned {
            evict: vec![pod_ref("a")],
            skipped: Vec::new(),
        },
        DrainProgress::Cordoned { already: true },
        DrainProgress::Evicting {
            pod: pod_ref("a"),
            attempt: 1,
        },
        DrainProgress::PodFailed {
            pod: pod_ref("a"),
            error: "still blocked at the timeout".into(),
        },
        DrainProgress::Finished(DrainSummary {
            node: "n1".into(),
            evicted: Vec::new(),
            skipped: Vec::new(),
            failed: vec![pod_ref("a")],
            dry_run: false,
        }),
    ]));
    let err = h
        .confirm_and_run(drain(DrainFlags::default()), ctx(Initiator::Ui))
        .unwrap_err();
    let DispatchError::Handler(err) = err else {
        panic!("a handler error: {err:?}");
    };
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.message().contains("shop/a"), "{err}");
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Failed, Initiator::Ui),
            (AuditOutcome::Failed, Initiator::Ui)
        ]
    );
}

#[test]
fn a_pod_that_blocks_the_drain_refuses_it_with_the_algorithms_reason() {
    let (h, _, port) = harness();
    port.script()
        .drain
        .push_err(oxikube_domain::OxiError::validation(format!(
            "cannot drain node n1: pod shop/lone {}",
            BlockReason::Unmanaged.explain()
        )));
    let err = h
        .confirm_and_run(drain(DrainFlags::default()), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(err.to_string().contains("force evicts it"), "{err}");
    assert!(h.writes("a").is_empty(), "nothing cordoned");
}

#[test]
fn cancelling_stops_the_drain_and_audits_it_as_cancelled_with_its_batch() {
    let (h, service, port) = harness();
    port.push(
        DrainRun::of([
            DrainProgress::Planned {
                evict: vec![pod_ref("a"), pod_ref("b")],
                skipped: Vec::new(),
            },
            DrainProgress::Cordoned { already: false },
            DrainProgress::Evicting {
                pod: pod_ref("a"),
                attempt: 1,
            },
            DrainProgress::Evicted { pod: pod_ref("a") },
            DrainProgress::Gone { pod: pod_ref("a") },
        ])
        .hold(),
    );
    let request = h.ask(drain(DrainFlags::default()), ctx(Initiator::Ui));
    let mut run = Box::pin(h.bus.dispatch(
        drain(DrainFlags::default()),
        ctx(Initiator::Ui).with_confirmation(Confirmation::typed(request.token, "n1")),
    ));
    assert!(run.as_mut().now_or_never().is_none(), "still draining");
    assert!(service.monitor().is_running(&id("a"), "n1"));
    // A second drain of the same node is refused while this one runs.
    let second = h.confirm_and_run(drain(DrainFlags::default()), ctx(Initiator::Ui));
    assert!(
        matches!(&second, Err(DispatchError::Handler(e)) if e.kind() == ErrorKind::Conflict),
        "{second:?}"
    );
    drop(run);
    assert!(!service.monitor().is_running(&id("a"), "n1"));

    let evictions = h
        .writes("a")
        .iter()
        .filter(|w| matches!(w, ResourceCall::Evict { .. }))
        .count();
    assert_eq!(evictions, 1, "nothing after the cancel");
    // Flush the backlog the drop queued.
    h.bus
        .guard()
        .audit()
        .flush()
        .now_or_never()
        .unwrap()
        .unwrap();
    let audit = h.audit();
    let drained: Vec<_> = audit
        .iter()
        .filter(|r| &*r.target.name == "n1")
        .map(|r| r.outcome)
        .collect();
    assert!(drained.contains(&AuditOutcome::Cancelled), "{audit:?}");
    let cancelled = audit
        .iter()
        .find(|r| r.outcome == AuditOutcome::Cancelled)
        .unwrap();
    let pod_record = audit.iter().find(|r| &*r.target.name == "a").unwrap();
    assert_eq!(batch_of(cancelled), batch_of(pod_record));
}

#[test]
fn read_only_switched_on_between_the_ask_and_the_answer_refuses_the_drain() {
    let (h, _, port) = harness();
    port.push(DrainRun::of([
        DrainProgress::Planned {
            evict: vec![pod_ref("a")],
            skipped: Vec::new(),
        },
        DrainProgress::Cordoned { already: true },
        DrainProgress::Evicting {
            pod: pod_ref("a"),
            attempt: 1,
        },
        DrainProgress::Evicted { pod: pod_ref("a") },
    ]));
    let request = h.ask(drain(DrainFlags::default()), ctx(Initiator::Ui));
    // The flag goes on while the dialog waits for the typed name.
    h.manager.set_read_only(&id("a"), true).unwrap();
    let err = h
        .dispatch(
            drain(DrainFlags::default()),
            ctx(Initiator::Ui).with_confirmation(Confirmation::typed(request.token, "n1")),
        )
        .unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }), "{err:?}");
    assert!(port.recorded_calls().is_empty());
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_dry_run_plans_and_writes_nothing() {
    let (h, _, port) = harness();
    let Ok(Outcome::Completed(output)) = h.confirm_and_run(
        drain(DrainFlags::default()),
        ctx(Initiator::Agent).with_dry_run(true),
    ) else {
        panic!("a dry run completes");
    };
    assert_eq!(output.data.unwrap()["outcome"], "dry_run");
    let [DrainCall::Drain { options, .. }] = &port.recorded_calls()[..] else {
        panic!("one drain");
    };
    assert!(options.dry_run);
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_target_that_is_not_a_node_is_refused() {
    let (h, _, port) = harness();
    let command = Command::NodeDrain {
        target: crate::testing::pod("a", "web"),
        flags: DrainFlags::default(),
    };
    let err = h
        .confirm_and_run(command, ctx(Initiator::Agent))
        .unwrap_err();
    assert!(err.to_string().contains("needs a node"), "{err}");
    assert!(port.recorded_calls().is_empty());
}

#[test]
fn the_drain_has_a_mutating_tool_stub() {
    let (h, _, _) = harness();
    let tool = h.bus.tool(CommandId::NODE_DRAIN).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.node_drain");
    assert!(tool.is_mutating() && tool.destructive_hint());
    assert_eq!(tool.risk, Some(Risk::High));
}

#[test]
fn the_flow_plans_from_the_pods_on_the_node_and_runs_with_the_typed_name() {
    let (h, service, port) = harness();
    let on_node = |b: oxikube_testkit::PodBuilder| b.node("n1").build();
    // The fake does not filter by field: the list is scripted, with a pod of another node the
    // flow must drop as a server that ignored the selector would send it.
    h.resources("a")
        .script()
        .list
        .push_ok(ListPage::complete(vec![
            on_node(
                pod()
                    .name("web-1")
                    .namespace("shop")
                    .controlled_by("ReplicaSet", "web"),
            ),
            on_node(
                pod()
                    .name("agent")
                    .namespace("kube-system")
                    .controlled_by("DaemonSet", "agent"),
            ),
            on_node(pod().name("lone").namespace("shop")),
            pod().name("elsewhere").namespace("shop").node("n2").build(),
        ]));
    let flow = DrainFlow::new(h.bus, service, "alice");
    let target = node_ref("a", "n1");
    let context = flow.open(&target).now_or_never().unwrap().expect("read");
    let names: Vec<_> = context.pods.iter().map(|p| p.name().to_owned()).collect();
    assert_eq!(names, ["agent", "lone", "web-1"], "the node's pods only");
    assert!(!context.cordoned);

    let plan = context.plan(&DrainFlags::default());
    assert_eq!(plan.evict.len(), 1);
    assert_eq!(plan.skipped.len(), 1, "the DaemonSet pod");
    assert_eq!(plan.blocked[0].reason, BlockReason::Unmanaged);
    let forced = context.plan(&DrainFlags {
        force: true,
        ..DrainFlags::default()
    });
    assert!(forced.blocked.is_empty());
    assert_eq!(forced.evict.len(), 2);

    // A wrong name typed: the guard refuses, nothing runs.
    let start = flow.start(&target, DrainFlags::default(), "n2".into());
    let err = start.run.now_or_never().unwrap().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
    assert!(port.recorded_calls().is_empty());

    // The right one: the drain runs and the follower sees it.
    port.push(two_pods());
    let start = flow.start(&target, DrainFlags::default(), "n1".into());
    let message = start.run.now_or_never().unwrap().expect("drained");
    assert!(message.starts_with("Drained node n1"), "{message}");
    let steps: Vec<_> = start.progress.collect().now_or_never().unwrap();
    assert_eq!(steps.len(), 11);
    assert_eq!(port.recorded_calls().len(), 1);
}
