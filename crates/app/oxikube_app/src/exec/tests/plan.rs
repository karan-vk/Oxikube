//! Single container opens directly, several make a picker with the right one preselected.

use oxikube_domain::{ErrorKind, Resource};

use super::{Fixture, annotated, pod_json};
use crate::exec::{
    ContainerPlan, DEFAULT_CONTAINER_ANNOTATION, PodContainers, container_to_open, plan_container,
};

fn containers(json: serde_json::Value) -> PodContainers {
    PodContainers::of(&Resource::from_json(json).unwrap())
}

#[test]
fn a_pod_with_one_container_opens_it_without_a_picker() {
    let pod = containers(pod_json(&[("app", true)]));
    assert_eq!(
        plan_container(&pod, None, None).unwrap(),
        ContainerPlan::Open("app".into())
    );
}

#[test]
fn several_containers_make_a_picker_with_the_default_container_preselected() {
    let pod = containers(annotated(
        pod_json(&[("app", true), ("proxy", true), ("logger", true)]),
        DEFAULT_CONTAINER_ANNOTATION,
        "proxy",
    ));
    let ContainerPlan::Pick(choices) = plan_container(&pod, None, None).unwrap() else {
        panic!("several containers must ask");
    };
    assert_eq!(choices.containers.len(), 3);
    assert_eq!(&*choices.preselected().name, "proxy", "the annotation wins");
}

#[test]
fn without_an_annotation_the_first_container_is_preselected() {
    let pod = containers(pod_json(&[("app", true), ("proxy", true)]));
    let ContainerPlan::Pick(choices) = plan_container(&pod, None, None).unwrap() else {
        panic!("several containers must ask");
    };
    assert_eq!(&*choices.preselected().name, "app");
}

#[test]
fn the_last_choice_for_the_pod_beats_the_default() {
    let pod = containers(annotated(
        pod_json(&[("app", true), ("proxy", true), ("logger", true)]),
        DEFAULT_CONTAINER_ANNOTATION,
        "proxy",
    ));
    let ContainerPlan::Pick(choices) = plan_container(&pod, None, Some("logger")).unwrap() else {
        panic!("several containers must ask");
    };
    assert_eq!(&*choices.preselected().name, "logger");
    let ContainerPlan::Pick(stale) = plan_container(&pod, None, Some("gone")).unwrap() else {
        panic!("several containers must ask");
    };
    assert_eq!(
        &*stale.preselected().name,
        "proxy",
        "a gone container is forgotten"
    );
}

#[test]
fn a_named_container_opens_as_it_is_and_an_unknown_one_is_not_found() {
    let pod = containers(pod_json(&[("app", true), ("proxy", true)]));
    assert_eq!(
        plan_container(&pod, Some("proxy"), None).unwrap(),
        ContainerPlan::Open("proxy".into())
    );
    let err = plan_container(&pod, Some("nope"), None).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("nope"));
}

#[test]
fn a_pod_with_nothing_to_open_is_a_validation_error() {
    let err = plan_container(&PodContainers::default(), None, None).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn a_command_without_a_pick_opens_the_default_container() {
    let pod = containers(annotated(
        pod_json(&[("app", true), ("proxy", true)]),
        DEFAULT_CONTAINER_ANNOTATION,
        "proxy",
    ));
    assert_eq!(&*container_to_open(&pod, None).unwrap(), "proxy");
    assert_eq!(&*container_to_open(&pod, Some("app")).unwrap(), "app");
}

#[tokio::test]
async fn the_service_plans_from_the_pod_it_reads_and_remembers_the_last_choice() {
    let f = Fixture::new();
    f.serve(pod_json(&[("app", true), ("proxy", true)]));
    let ContainerPlan::Pick(first) = f.service.plan(&f.pod(), None).await.unwrap() else {
        panic!("two containers ask");
    };
    assert_eq!(&*first.preselected().name, "app");

    f.service.remember(&f.pod(), "proxy");
    assert_eq!(f.service.last_container(&f.pod()).as_deref(), Some("proxy"));
    f.serve(pod_json(&[("app", true), ("proxy", true)]));
    let ContainerPlan::Pick(second) = f.service.plan(&f.pod(), None).await.unwrap() else {
        panic!("two containers ask");
    };
    assert_eq!(
        &*second.preselected().name,
        "proxy",
        "remembered for the session"
    );

    f.serve(pod_json(&[("app", true)]));
    assert_eq!(
        f.service.plan(&f.pod(), None).await.unwrap(),
        ContainerPlan::Open("app".into()),
        "one container opens directly"
    );
}

#[tokio::test]
async fn the_remembered_choices_are_bounded() {
    let f = Fixture::new();
    for n in 0..1_000 {
        let pod = crate::testing::pod("a", &format!("pod-{n}"));
        f.service.remember(&pod, "app");
    }
    let newest = crate::testing::pod("a", "pod-999");
    assert_eq!(f.service.last_container(&newest).as_deref(), Some("app"));
    assert!(
        f.service
            .last_container(&crate::testing::pod("a", "pod-0"))
            .is_none(),
        "old choices are forgotten, not kept for ever"
    );
}
