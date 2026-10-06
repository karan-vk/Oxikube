//! Read-only sessions deny every mutating command, for every initiator, before any
//! request, and name the cluster; reads and the privileged toggle still run.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::{ErrorKind, OxiError};

use crate::command_bus::{DispatchError, Outcome};
use crate::testing::{Harness, ctx, id, node, pod_delete, view_logs};

#[test]
fn read_only_session_denies_a_mutating_command_and_names_the_cluster() {
    let h = Harness::new();
    h.connect("a", true);

    let err = h
        .dispatch(pod_delete("a", "web-0"), ctx(Initiator::Ui))
        .unwrap_err();
    match &err {
        DispatchError::ReadOnly { cluster, context } => {
            assert_eq!(cluster, &id("a"));
            assert_eq!(context.as_str(), "a");
        }
        other => panic!("expected ReadOnly, got {other:?}"),
    }
    assert_eq!(err.read_only_cluster(), Some(&id("a")));
    assert_eq!(err.to_string(), "cluster a is read-only");
    let oxi = OxiError::from(err);
    assert_eq!(oxi.kind(), ErrorKind::Forbidden);
    assert!(oxi.message().contains("read-only"));

    assert!(h.writes("a").is_empty(), "no request reached the cluster");
    assert!(h.calls().is_empty(), "the handler never ran");
    assert_eq!(h.outcomes(), [(AuditOutcome::Denied, Initiator::Ui)]);
}

#[test]
fn every_initiator_gets_the_same_read_only_block() {
    let h = Harness::new();
    h.connect("a", true);
    for initiator in Initiator::ALL {
        for cmd in [
            pod_delete("a", "web-0"),
            Command::NodeDrain {
                target: node("a", "worker-1"),
                force: false,
            },
        ] {
            let err = h.dispatch(cmd, ctx(initiator)).unwrap_err();
            assert!(
                matches!(err, DispatchError::ReadOnly { .. }),
                "{initiator}: {err:?}"
            );
        }
    }
    assert!(h.writes("a").is_empty());
    let audit = h.audit();
    assert_eq!(audit.len(), Initiator::ALL.len() * 2);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
    for initiator in Initiator::ALL {
        assert_eq!(audit.iter().filter(|r| r.initiator == initiator).count(), 2);
    }
}

#[test]
fn read_only_session_allows_a_read_command() {
    let h = Harness::new();
    h.connect("a", true);

    let out = h
        .dispatch(view_logs("a", "web-0"), ctx(Initiator::Agent))
        .unwrap();
    assert!(matches!(out, Outcome::Completed(_)));
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].cluster, Some(id("a")));
    assert!(!calls[0].mutation, "read handlers get no write permit");
    assert!(h.audit().is_empty(), "reads are not audited");
}

#[test]
fn read_only_only_blocks_its_own_cluster() {
    let h = Harness::new();
    h.connect("a", true);
    h.connect("b", false);
    h.allow_deletes("b", 1);

    assert!(
        h.dispatch(pod_delete("a", "web-0"), ctx(Initiator::Ui))
            .is_err()
    );
    let out = h.confirm_and_run(pod_delete("b", "web-0"), ctx(Initiator::Ui));
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert!(h.writes("a").is_empty());
    assert_eq!(h.writes("b").len(), 1);
}

#[test]
fn turning_read_only_on_blocks_a_pending_confirmation() {
    let h = Harness::new();
    h.connect("a", false);
    let request = h.ask(pod_delete("a", "web-0"), ctx(Initiator::Ui));

    h.manager.set_read_only(&id("a"), true).unwrap();
    let err = h
        .dispatch(
            pod_delete("a", "web-0"),
            ctx(Initiator::Ui).with_confirmation(crate::Confirmation::simple(request.token)),
        )
        .unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }));
    assert!(h.writes("a").is_empty());
}

#[test]
fn privileged_toggle_runs_on_a_read_only_cluster_for_people_only() {
    let h = Harness::new();
    h.connect("a", true);
    let lift = || Command::ClusterToggleReadOnly {
        cluster: id("a"),
        read_only: Some(false),
    };

    for initiator in [Initiator::Agent, Initiator::Plugin] {
        let err = h.dispatch(lift(), ctx(initiator)).unwrap_err();
        assert!(
            matches!(err, DispatchError::NotPermitted { initiator: i, .. } if i == initiator),
            "{err:?}"
        );
        assert_eq!(OxiError::from(err).kind(), ErrorKind::Forbidden);
    }
    assert!(h.manager.get(&id("a")).unwrap().read_only());

    h.dispatch(lift(), ctx(Initiator::Ui)).unwrap();
    assert!(!h.manager.get(&id("a")).unwrap().read_only());
}

#[test]
fn a_cluster_without_a_session_is_refused_closed() {
    let h = Harness::new();
    let err = h
        .dispatch(pod_delete("a", "web-0"), ctx(Initiator::Command))
        .unwrap_err();
    assert!(
        matches!(err, DispatchError::NoSession(ref c) if *c == id("a")),
        "{err:?}"
    );
    assert_eq!(h.outcomes(), [(AuditOutcome::Denied, Initiator::Command)]);
}

#[test]
fn a_disconnected_session_fails_without_asking_to_confirm() {
    let h = Harness::new();
    h.connect("a", false);
    h.manager.disconnect(&id("a")).unwrap();
    let err = h
        .dispatch(pod_delete("a", "web-0"), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(err, DispatchError::NotConnected { .. }), "{err:?}");
    assert_eq!(OxiError::from(err).kind(), ErrorKind::Conflict);
    assert_eq!(h.guard_pending(), 0);
    assert_eq!(h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}

impl Harness {
    fn guard_pending(&self) -> usize {
        self.bus.guard().pending_confirmations()
    }
}
