//! `RollbackRunner`: what a dialog calls.

use oxikube_domain::ErrorKind;

use super::{Rollouts, deployment_ref};
use crate::rollout::RollbackRunner;

fn runner(t: &Rollouts) -> RollbackRunner {
    RollbackRunner::new(t.h.bus.clone(), "alice")
}

#[test]
fn the_runner_answers_the_simple_confirmation_and_reports() {
    let t = Rollouts::new(false);
    t.answer_patches();
    let report = futures::executor::block_on(runner(&t).run(&deployment_ref(), Some(2))).unwrap();
    assert_eq!(report.to_revision, Some(2));
    assert!(!report.already_current);
    assert_eq!(
        report.message,
        "Rolled back Deployment shop/web from revision 3 to revision 2"
    );
    // The guard's dry run before it asked, then the rollback.
    assert_eq!(t.h.audit().len(), 2);
    assert!(t.h.audit()[0].dry_run && !t.h.audit()[1].dry_run);
    assert_eq!(
        t.h.audit()[1].initiator,
        oxikube_domain::audit::Initiator::Ui
    );
}

#[test]
fn already_at_the_revision_is_reported_without_a_write() {
    let t = Rollouts::new(false);
    let report = futures::executor::block_on(runner(&t).run(&deployment_ref(), Some(3))).unwrap();
    assert!(report.already_current);
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_read_only_cluster_refuses_the_runner() {
    let t = Rollouts::new(true);
    let error = futures::executor::block_on(runner(&t).run(&deployment_ref(), None)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(t.port().recorded_calls().is_empty());
}
