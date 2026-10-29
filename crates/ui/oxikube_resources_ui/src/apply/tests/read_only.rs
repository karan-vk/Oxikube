//! A read-only cluster: the lock, Apply disabled with the reason, nothing sent.

use gpui::TestAppContext;

use super::{CM_A, Rig, seed};
use crate::apply::Stage;
use crate::table::tests::fixture::cluster;

#[gpui::test]
fn a_read_only_cluster_shows_the_lock_loads_and_plans_but_sends_nothing(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    rig.f.sessions.set_read_only(&cluster(), true).expect("set");
    seed(&rig, "/m", &[("a.yaml", CM_A)]);
    rig.open("/m");
    assert!(rig.is_laid_out("apply-lock"));
    assert_eq!(
        rig.read(|v| v.model().stage()),
        Stage::Review,
        "the plan is shown"
    );
    assert_eq!(
        rig.words(),
        ["waiting"],
        "the guard refuses a dry run here, so none is asked for"
    );
    assert!(rig.writes().is_empty(), "no request reached the cluster");
    let reason = rig.read(|v| v.apply_blocked()).expect("blocked");
    assert!(reason.contains("read-only"), "{reason}");
    assert!(rig.is_laid_out("apply-blocked"), "the reason is on screen");
    // The button is disabled: a click does not ask for a confirmation.
    rig.click("apply-apply");
    assert!(!rig.is_laid_out("apply-confirmation"));
    assert!(rig.writes().is_empty());
}

#[gpui::test]
fn read_only_turned_on_after_the_review_blocks_the_apply_in_the_guard(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    seed(&rig, "/m", &[("a.yaml", CM_A)]);
    rig.open("/m");
    rig.click("apply-apply");
    assert!(rig.is_laid_out("apply-confirmation"));
    let before = rig.writes().len();
    rig.f.sessions.set_read_only(&cluster(), true).expect("set");
    rig.click("apply-confirm");
    assert_eq!(
        rig.writes().len(),
        before,
        "nothing was sent after the lock"
    );
    assert!(rig.words().iter().all(|w| *w != "created" || before > 0));
    assert!(!rig.read(|v| v.model().busy()));
}
