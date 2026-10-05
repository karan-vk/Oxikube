//! Pod and port resolution: named `targetPort`s, replica choice, why nothing matched.

use oxikube_domain::{ErrorKind, ForwardPort};

use super::fakes::{pod, service, service_port};
use crate::remote::portforward::plan::{Plan, PodInfo, PodSelector, Target, TargetPort};

fn web_service(target: TargetPort) -> Plan {
    let service = service(
        &[("app", "web")],
        vec![
            service_port(Some("web"), 80, target),
            service_port(None, 443, TargetPort::Number(8443)),
        ],
    );
    Plan::service("default", "web", service, &ForwardPort::Number(80)).expect("plan")
}

fn target(pod: &str, port: u16) -> Option<Target> {
    Some(Target {
        pod: pod.to_owned(),
        port,
    })
}

#[test]
fn a_named_target_port_resolves_through_the_chosen_pods_container_ports() {
    let plan = web_service(TargetPort::Name("http".into()));
    assert_eq!(plan.pick(&[pod("web-1", 1)]), target("web-1", 8080));

    // Another replica that moved the port: each pod resolves on its own.
    let mut other = pod("web-2", 2);
    other.ports[0].number = 3000;
    assert_eq!(plan.target_on(&other), target("web-2", 3000));
}

#[test]
fn a_pod_without_the_named_port_cannot_serve() {
    let plan = web_service(TargetPort::Name("metrics".into()));
    assert_eq!(plan.pick(&[pod("web-1", 1)]), None);
    let err = plan.why_no_target(&[pod("web-1", 1)]);
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("metrics"), "{}", err.message());
}

#[test]
fn a_numeric_or_missing_target_port_is_used_as_is() {
    assert_eq!(
        web_service(TargetPort::Number(9090)).pick(&[pod("web-1", 1)]),
        target("web-1", 9090)
    );
}

#[test]
fn the_service_port_is_matched_by_number_or_name_and_unknown_ones_are_refused() {
    let svc = || {
        service(
            &[("app", "web")],
            vec![
                service_port(Some("web"), 80, TargetPort::Number(8080)),
                service_port(None, 443, TargetPort::Number(8443)),
            ],
        )
    };
    let by_name = Plan::service("default", "web", svc(), &ForwardPort::Named("web".into()));
    assert!(by_name.is_ok());
    let missing = Plan::service("default", "web", svc(), &ForwardPort::Number(81))
        .expect_err("81 is not a service port");
    assert_eq!(missing.kind(), ErrorKind::Validation);
    assert!(missing.message().contains("80/web"), "lists what is served");
}

#[test]
fn a_service_without_a_selector_is_refused() {
    let svc = service(&[], vec![service_port(None, 80, TargetPort::Number(80))]);
    let err =
        Plan::service("default", "ext", svc, &ForwardPort::Number(80)).expect_err("no selector");
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn a_service_picks_the_oldest_ready_pod_and_skips_the_rest() {
    let plan = web_service(TargetPort::Number(8080));
    let mut not_ready = pod("web-0", 0);
    not_ready.ready = false;
    let mut terminating = pod("web-1", 1);
    terminating.terminating = true;
    let mut pending = pod("web-2", 2);
    pending.running = false;
    let pods: Vec<PodInfo> = vec![
        pod("web-4", 40),
        not_ready,
        terminating,
        pending,
        pod("web-3", 30),
        pod("web-5", 30),
    ];
    assert_eq!(
        plan.pick(&pods),
        target("web-3", 8080),
        "oldest, name breaks the tie"
    );
}

#[test]
fn a_pod_forward_does_not_need_readiness_but_needs_running() {
    let plan = Plan::pod("default", "web-1", ForwardPort::Number(80));
    let mut sick = pod("web-1", 1);
    sick.ready = false;
    assert_eq!(plan.pick(&[sick.clone()]), target("web-1", 80));
    sick.running = false;
    assert_eq!(plan.pick(&[sick.clone()]), None);
    assert_eq!(
        plan.why_no_target(&[sick.clone()]).kind(),
        ErrorKind::Conflict
    );
    sick.running = true;
    sick.terminating = true;
    assert_eq!(plan.pick(&[sick]), None);
}

#[test]
fn a_named_pod_port_is_looked_up_and_a_missing_one_names_the_port() {
    let plan = Plan::pod("default", "web-1", ForwardPort::Named("http".into()));
    assert_eq!(plan.pick(&[pod("web-1", 1)]), target("web-1", 8080));
    let plan = Plan::pod("default", "web-1", ForwardPort::Named("grpc".into()));
    let err = plan.why_no_target(&[pod("web-1", 1)]);
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("grpc"));
}

#[test]
fn a_missing_pod_is_not_found_and_an_empty_service_is_retryable() {
    let plan = Plan::pod("default", "web-1", ForwardPort::Number(80));
    assert_eq!(plan.why_no_target(&[]).kind(), ErrorKind::NotFound);

    let err = web_service(TargetPort::Number(80)).why_no_target(&[]);
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.is_retryable());
}

#[test]
fn selectors_render_as_query_values() {
    assert_eq!(
        PodSelector::Name("web-1".into())
            .field_selector()
            .as_deref(),
        Some("metadata.name=web-1")
    );
    let labels = PodSelector::Labels(
        [("app", "web"), ("tier", "fe")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    );
    assert_eq!(labels.label_selector().as_deref(), Some("app=web,tier=fe"));
    assert_eq!(labels.field_selector(), None);
}
