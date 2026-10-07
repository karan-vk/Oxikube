//! Debug containers (E09-S10): the spec built from the request, defaults, unique names, the
//! failure paths (rejected patch, never starts, read-only) and the hand-off to the terminal.

use futures::StreamExt as _;
use oxikube_domain::command::{DEFAULT_DEBUG_COMMAND, DEFAULT_DEBUG_IMAGE};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{ErrorKind, OxiError, Resource};
use oxikube_ports::BackendEvent;
use oxikube_testkit::ExecPortCall;
use serde_json::{Value, json};
use std::collections::HashSet;

use super::{Fixture, annotated, pod_json};
use crate::exec::{
    DEFAULT_DEBUG_START_TIMEOUT, DebugRequest, ExecService, check_name, plan_debug, split_command,
};
use crate::guard::Mutation;
use crate::testing::{Harness, id};

/// The pod `web-0` with `app` and `proxy` running.
fn two() -> Value {
    pod_json(&[("app", true), ("proxy", true)])
}

fn request(f: &Fixture) -> DebugRequest {
    DebugRequest::new(f.pod(), DEFAULT_DEBUG_IMAGE)
}

fn resource(json: Value) -> Resource {
    Resource::from_json(json).expect("a pod")
}

fn permit(f: &Fixture, dry_run: bool) -> Mutation {
    Mutation::new(id("a"), f.resources.clone(), dry_run)
}

fn with_ephemeral(mut pod: Value, name: &str) -> Value {
    pod["spec"]["ephemeralContainers"] = json!([{"name": name, "image": "busybox"}]);
    pod
}

#[test]
fn defaults_are_busybox_sh_and_the_pods_default_container() {
    let plan = plan_debug(&request(&Fixture::new()), &resource(two())).unwrap();
    assert_eq!(plan.image, "busybox");
    assert_eq!(plan.command, [DEFAULT_DEBUG_COMMAND]);
    assert_eq!(&*plan.target, "app", "the first container, like kubectl");
    let annotated = annotated(two(), "kubectl.kubernetes.io/default-container", "proxy");
    let plan = plan_debug(&request(&Fixture::new()), &resource(annotated)).unwrap();
    assert_eq!(
        &*plan.target, "proxy",
        "the default-container annotation wins"
    );
}

#[test]
fn custom_values_are_kept_and_the_name_is_unique_and_five_characters_long() {
    let f = Fixture::new();
    let mut custom = request(&f);
    custom.image = " nicolaka/netshoot ".into();
    custom.target_container = Some("proxy".into());
    custom.command = vec!["bash".into(), "-l".into()];
    custom.name = Some("netshoot".into());
    let plan = plan_debug(&custom, &resource(two())).unwrap();
    assert_eq!(plan.image, "nicolaka/netshoot");
    assert_eq!(&*plan.target, "proxy");
    assert_eq!(plan.command, ["bash", "-l"]);
    assert_eq!(plan.name, "netshoot");

    let mut names = HashSet::new();
    for _ in 0..200 {
        let name = plan_debug(&request(&f), &resource(two())).unwrap().name;
        let suffix = name.strip_prefix("debugger-").expect("debugger-xxxxx");
        assert_eq!(suffix.len(), 5, "{name}");
        assert!(
            check_name(&name).is_ok(),
            "{name} is a valid container name"
        );
        names.insert(name);
    }
    assert!(names.len() > 190, "names are random: {}", names.len());
}

#[test]
fn a_name_in_use_a_missing_target_a_blank_image_and_a_finished_pod_are_refused() {
    let f = Fixture::new();
    let taken = with_ephemeral(two(), "debugger-old");
    let mut named = request(&f);
    named.name = Some("debugger-old".into());
    let err = plan_debug(&named, &resource(taken.clone())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");
    named.name = Some("app".into());
    let err = plan_debug(&named, &resource(two())).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::Conflict,
        "an existing container's name: {err}"
    );
    named.name = Some("Not Valid".into());
    assert_eq!(
        plan_debug(&named, &resource(two())).unwrap_err().kind(),
        ErrorKind::Validation
    );

    let mut missing = request(&f);
    missing.target_container = Some("db".into());
    let err = plan_debug(&missing, &resource(two())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
    missing.target_container = Some("debugger-old".into());
    assert_eq!(
        plan_debug(&missing, &resource(taken)).unwrap_err().kind(),
        ErrorKind::NotFound,
        "an ephemeral container is no target"
    );

    for image in ["", "   ", "two words"] {
        let mut blank = request(&f);
        blank.image = image.into();
        let err = plan_debug(&blank, &resource(two())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation, "{image:?}");
    }

    let mut finished = two();
    finished["status"]["phase"] = json!("Succeeded");
    let err = plan_debug(&request(&f), &resource(finished)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.message().contains("finished"), "{err}");
}

#[test]
fn a_command_without_a_program_is_refused_before_anything_is_read() {
    let f = Fixture::new();
    let mut blank = request(&f);
    blank.command = vec!["  ".into()];
    let err = blank.check_fields().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn the_command_field_splits_like_a_shell_would() {
    assert_eq!(split_command("sh").unwrap(), ["sh"]);
    assert_eq!(split_command("  ").unwrap(), Vec::<String>::new());
    assert_eq!(
        split_command(r#"sh -c "ls -l /" 'a b' c\ d "" "#).unwrap(),
        ["sh", "-c", "ls -l /", "a b", "c d", ""]
    );
    assert!(split_command("sh -c 'oops").is_err());
    assert!(split_command("sh \\").is_err());
}

#[test]
fn names_are_dns_labels() {
    for good in ["a", "debugger-x1y2z", "a-b-c", &"a".repeat(63)] {
        assert!(check_name(good).is_ok(), "{good}");
    }
    for bad in ["", "-a", "a-", "A", "a_b", "a.b", &"a".repeat(64)] {
        assert!(check_name(bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn the_dialog_starts_with_the_pods_containers_and_the_last_image_of_the_cluster() {
    let f = Fixture::new();
    f.serve(with_ephemeral(two(), "debugger-old"));
    let defaults = f.service.debug_defaults(&f.pod()).await.unwrap();
    assert_eq!(defaults.image, "busybox");
    assert_eq!(defaults.command, "sh");
    let names: Vec<_> = defaults.targets.iter().map(|c| &*c.name).collect();
    assert_eq!(
        names,
        ["app", "proxy"],
        "an ephemeral container is no target"
    );
    assert_eq!(defaults.target, 0);

    // The image used last in this cluster is the next default.
    f.serve(two());
    let mut custom = request(&f);
    custom.image = "nicolaka/netshoot".into();
    f.service
        .open_debug(&permit(&f, false), &custom)
        .await
        .unwrap();
    f.serve(two());
    let defaults = f.service.debug_defaults(&f.pod()).await.unwrap();
    assert_eq!(defaults.image, "nicolaka/netshoot");
    assert_eq!(
        f.service.debug.last_image(&f.pod().cluster).as_deref(),
        Some("nicolaka/netshoot")
    );
}

#[tokio::test]
async fn opening_patches_with_the_planned_spec_and_waits_for_the_terminal_to_claim_it() {
    let f = Fixture::new();
    f.serve(two());
    let mut asked = request(&f);
    asked.target_container = Some("proxy".into());
    let opened = f
        .service
        .open_debug(&permit(&f, false), &asked)
        .await
        .unwrap();
    assert!(!opened.dry_run);
    let calls = f.exec.recorded_calls();
    let [ExecPortCall::CreateDebugContainer(spec)] = calls.as_slice() else {
        panic!("one debug container, got {calls:?}");
    };
    assert_eq!(spec.image, "busybox");
    assert_eq!(spec.name.as_deref(), Some(opened.plan.name.as_str()));
    assert_eq!(spec.target_container.as_deref(), Some("proxy"));
    assert_eq!(spec.command, ["sh"]);
    assert_eq!(spec.start_timeout, DEFAULT_DEBUG_START_TIMEOUT);
    assert_eq!(f.service.unclaimed_debug_sessions(), 1);

    // The terminal of the container attaches: it gets the session that was opened, with the
    // notice first, and no second attach is made.
    let name = opened.plan.name.clone();
    let backend = f.service.attach(&f.pod(), Some(&name)).await.unwrap();
    assert_eq!(f.service.unclaimed_debug_sessions(), 0);
    assert_eq!(f.exec.recorded_calls().len(), 1, "no second attach");
    let Some(BackendEvent::Output(first)) = backend.output_stream().next().await else {
        panic!("the notice comes first");
    };
    let text = String::from_utf8_lossy(&first);
    assert!(
        text.contains(&name) && text.contains("busybox") && text.contains("proxy"),
        "{text:?}"
    );

    // A reconnect is a new attach to the same container; it never adds another one.
    f.service.attach(&f.pod(), Some(&name)).await.unwrap();
    let calls = f.exec.recorded_calls();
    let [_, ExecPortCall::Attach(target)] = calls.as_slice() else {
        panic!("a plain attach, got {calls:?}");
    };
    assert_eq!(target.container.as_deref(), Some(name.as_str()));
    assert_eq!(
        f.service.last_container(&f.pod()).as_deref(),
        Some(name.as_str())
    );
}

#[tokio::test]
async fn a_dry_run_plans_and_changes_nothing() {
    let f = Fixture::new();
    f.serve(two());
    let opened = f
        .service
        .open_debug(&permit(&f, true), &request(&f))
        .await
        .unwrap();
    assert!(opened.dry_run);
    assert!(f.exec.recorded_calls().is_empty());
    assert_eq!(f.service.unclaimed_debug_sessions(), 0);
}

#[tokio::test]
async fn a_rejected_patch_says_why_and_a_container_that_never_starts_times_out() {
    let f = Fixture::new();
    for (error, kind) in [
        (
            OxiError::forbidden(
                "pods \"web-0\" is forbidden: violates PodSecurity \"restricted:latest\"",
            ),
            ErrorKind::Forbidden,
        ),
        (
            OxiError::unsupported("the cluster does not support ephemeral containers"),
            ErrorKind::Unsupported,
        ),
        (
            OxiError::timeout("debug container debugger-x did not start within 60s"),
            ErrorKind::Timeout,
        ),
    ] {
        f.serve(two());
        let message = error.message().to_owned();
        f.exec.script().create_debug_container.push_err(error);
        let err = f
            .service
            .open_debug(&permit(&f, false), &request(&f))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), kind);
        assert_eq!(err.message(), message, "the API's message reaches the user");
    }
    assert_eq!(
        f.service.unclaimed_debug_sessions(),
        0,
        "nothing waits for a terminal"
    );
    assert!(
        f.service.debug.last_image(&f.pod().cluster).is_none(),
        "a failure remembers nothing"
    );
}

#[tokio::test]
async fn a_cluster_that_went_read_only_is_refused_before_the_patch() {
    let h = Harness::new();
    h.connect("a", true);
    let ports = h.connector.ports_for(&id("a"));
    let service = ExecService::new(h.manager.clone());
    ports.resources.script().get.push_ok(resource(two()));
    let pod: ResourceRef = crate::testing::pod("a", "web-0");
    let permit = Mutation::new(id("a"), ports.resources.clone(), false);
    let err = service
        .open_debug(&permit, &DebugRequest::new(pod, "busybox"))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden, "{err}");
    assert!(ports.exec.recorded_calls().is_empty());
}

#[tokio::test]
async fn a_permit_for_another_cluster_is_not_accepted() {
    let f = Fixture::new();
    f.serve(two());
    let other = Mutation::new(id("b"), f.resources.clone(), false);
    let err = f
        .service
        .open_debug(&other, &request(&f))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(f.exec.recorded_calls().is_empty());
}

#[tokio::test]
async fn a_session_nobody_claims_can_be_discarded_and_only_a_few_wait() {
    let f = Fixture::new();
    f.serve(two());
    let opened = f
        .service
        .open_debug(&permit(&f, false), &request(&f))
        .await
        .unwrap();
    f.service.discard_debug(&f.pod(), &opened.plan.name);
    assert_eq!(f.service.unclaimed_debug_sessions(), 0);
    for _ in 0..12 {
        f.serve(two());
        f.service
            .open_debug(&permit(&f, false), &request(&f))
            .await
            .unwrap();
    }
    assert_eq!(
        f.service.unclaimed_debug_sessions(),
        8,
        "the oldest are dropped, ending them"
    );
}

#[test]
fn a_command_makes_the_request_and_back() {
    let f = Fixture::new();
    let command = oxikube_domain::command::Command::PodDebug {
        target: f.pod(),
        image: "  ".into(),
        target_container: Some("".into()),
        command: vec![],
        name: Some(" ".into()),
    };
    let request = DebugRequest::from_command(&command).unwrap();
    assert_eq!(request.image, "busybox", "a blank image is the default");
    assert_eq!(request.target_container, None);
    assert_eq!(request.name, None);
    assert!(DebugRequest::from_command(&oxikube_domain::command::Command::PaletteToggle).is_err());
}
