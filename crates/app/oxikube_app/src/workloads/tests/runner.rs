//! `ScaleRunner`: the dialog's way through the bus.

use oxikube_domain::ErrorKind;

use super::{deployment, scale};
use crate::testing::Harness;
use crate::workloads::ScaleRunner;

fn run(h: &Harness, replicas: u32) -> oxikube_domain::OxiResult<crate::workloads::ScaleReport> {
    let runner = ScaleRunner::new(h.bus.clone(), "alice");
    futures::executor::block_on(runner.run(&deployment("a"), replicas))
}

#[test]
fn the_runner_answers_the_simple_confirmation_and_reports() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").script().get_scale.push_ok(scale(2, 2));
    h.resources("a").script().scale.push_ok(scale(4, 2));
    let report = run(&h, 4).unwrap();
    assert_eq!(report.after, Some(4));
    assert_eq!(
        report.message,
        "Scaled Deployment shop/web from 2 to 4 replicas"
    );
    assert_eq!(h.audit().len(), 1);
    assert_eq!(h.audit()[0].initiator, oxikube_domain::audit::Initiator::Ui);
    assert_eq!(&*h.audit()[0].who, "alice");
}

#[test]
fn scaling_to_zero_is_one_simple_confirmation_too() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").script().get_scale.push_ok(scale(2, 2));
    h.resources("a").script().scale.push_ok(scale(0, 2));
    assert_eq!(run(&h, 0).unwrap().after, Some(0));
}

#[test]
fn a_read_only_cluster_says_forbidden_and_sends_nothing() {
    let h = Harness::with_workloads();
    h.connect("a", true);
    let error = run(&h, 3).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(h.resources("a").recorded_calls().is_empty());
}

#[test]
fn the_servers_refusal_comes_back_in_its_own_words() {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").script().get_scale.push_ok(scale(2, 2));
    h.resources("a")
        .script()
        .scale
        .push_err(oxikube_domain::OxiError::forbidden(
            "admission webhook \"quota\" denied the request",
        ));
    let error = run(&h, 50).unwrap_err();
    assert!(error.message().contains("quota"), "{}", error.message());
}
