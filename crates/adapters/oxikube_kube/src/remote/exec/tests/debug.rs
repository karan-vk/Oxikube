//! The debug container flow: patch, wait, attach.

use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_ports::ExecOptions;
use oxikube_testkit::fakes::{ExecScript, ExecStreamCall, FakeExecStreamPort};

use super::fakes::{Call, FakePods};
use crate::remote::exec::debug::attach_debug;
use crate::remote::exec::wait::Container;
use crate::subresource::EphemeralContainerSpec;

fn spec() -> EphemeralContainerSpec {
    EphemeralContainerSpec {
        name: "dbg".into(),
        image: "busybox:1.37".into(),
        stdin: true,
        tty: true,
        target_container: Some("app".into()),
        ..EphemeralContainerSpec::default()
    }
}

#[tokio::test]
async fn it_patches_waits_then_attaches_to_the_new_container() {
    let (pods, _deleted) = FakePods::new();
    let exec = FakeExecStreamPort::new();
    exec.script().attach.push(Ok(ExecScript::new()));
    let session = attach_debug(&exec, &pods, "ns", "p", &spec(), Duration::from_secs(1))
        .await
        .expect("attached");
    assert!(session.stdin.is_some() && session.resize.is_some());

    let calls = pods.calls();
    let Call::Patch { pod, body } = &calls[0] else {
        panic!("expected a patch first, got {calls:?}");
    };
    assert_eq!(pod, "p");
    let container = &body["spec"]["ephemeralContainers"][0];
    assert_eq!(container["name"], "dbg");
    assert_eq!(container["targetContainerName"], "app");
    assert_eq!(
        calls[1],
        Call::Wait {
            pod: "p".into(),
            container: Container::Ephemeral("dbg".into())
        }
    );
    let ExecStreamCall::Attach { options, .. } = &exec.recorded_calls()[0] else {
        panic!("expected an attach");
    };
    assert_eq!(*options, ExecOptions::interactive().container("dbg"));
}

#[tokio::test]
async fn without_a_tty_stderr_is_attached_separately() {
    let (pods, _deleted) = FakePods::new();
    let exec = FakeExecStreamPort::new();
    exec.script().attach.push(Ok(ExecScript::new()));
    let spec = EphemeralContainerSpec {
        stdin: false,
        tty: false,
        ..spec()
    };
    attach_debug(&exec, &pods, "ns", "p", &spec, Duration::from_secs(1))
        .await
        .expect("attached");
    let ExecStreamCall::Attach { options, .. } = &exec.recorded_calls()[0] else {
        panic!("expected an attach");
    };
    assert!(options.stderr && options.stdout && !options.stdin && !options.tty);
    assert!(options.is_valid());
}

#[tokio::test]
async fn a_rejected_patch_stops_before_waiting_or_attaching() {
    let (pods, _deleted) = FakePods::new();
    *pods.patch_error.lock() = Some((ErrorKind::Forbidden, "no"));
    let exec = FakeExecStreamPort::new();
    let err = attach_debug(&exec, &pods, "ns", "p", &spec(), Duration::from_secs(1))
        .await
        .expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert_eq!(pods.calls().len(), 1);
    assert!(exec.recorded_calls().is_empty());
}

#[tokio::test]
async fn a_container_that_cannot_start_is_not_attached() {
    let (pods, _deleted) = FakePods::new();
    *pods.wait_error.lock() = Some((ErrorKind::Conflict, "ErrImagePull"));
    let exec = FakeExecStreamPort::new();
    let err = attach_debug(&exec, &pods, "ns", "p", &spec(), Duration::from_secs(1))
        .await
        .expect_err("fails");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(exec.recorded_calls().is_empty());
}

#[tokio::test]
async fn an_incomplete_spec_is_refused_before_the_patch() {
    let (pods, _deleted) = FakePods::new();
    let exec = FakeExecStreamPort::new();
    for bad in [
        EphemeralContainerSpec {
            name: String::new(),
            ..spec()
        },
        EphemeralContainerSpec {
            image: " ".into(),
            ..spec()
        },
    ] {
        let err = attach_debug(&exec, &pods, "ns", "p", &bad, Duration::from_secs(1))
            .await
            .expect_err("refused");
        assert_eq!(err.kind(), ErrorKind::Validation);
    }
    assert!(pods.calls().is_empty());
}
