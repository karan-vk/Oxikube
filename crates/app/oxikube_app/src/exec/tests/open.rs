//! Attach, exec, and the failures common to every way of opening a session.

use futures::StreamExt as _;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::BackendEvent;
use oxikube_testkit::ExecPortCall;

use super::{Fixture, pod_json};
use crate::exec::notice_line_for_tests;

#[tokio::test]
async fn attach_opens_the_main_process_with_a_notice() {
    let f = Fixture::new();
    let backend = f.service.attach(&f.pod(), Some("app")).await.unwrap();
    let calls = f.exec.recorded_calls();
    let [ExecPortCall::Attach(target)] = calls.as_slice() else {
        panic!("one attach, got {calls:?}");
    };
    assert_eq!(target.container.as_deref(), Some("app"));
    assert!(target.tty && target.stdin);
    let Some(BackendEvent::Output(first)) = backend.output_stream().next().await else {
        panic!("the notice comes first");
    };
    let text = String::from_utf8_lossy(&first);
    assert!(text.contains("attached to web-0/app"), "{text:?}");
}

#[tokio::test]
async fn attach_without_a_container_attaches_to_the_default_one() {
    let f = Fixture::new();
    f.serve(pod_json(&[("app", true), ("proxy", true)]));
    f.service.attach(&f.pod(), None).await.unwrap();
    let calls = f.exec.recorded_calls();
    let [ExecPortCall::Attach(target)] = calls.as_slice() else {
        panic!("one attach");
    };
    assert_eq!(target.container.as_deref(), Some("app"));
}

#[tokio::test]
async fn exec_runs_the_argv_as_given_and_rejects_an_empty_one() {
    let f = Fixture::new();
    let argv = vec!["psql".to_owned(), "-U".to_owned(), "app".to_owned()];
    f.service.exec(&f.pod(), Some("db"), &argv).await.unwrap();
    let calls = f.exec.recorded_calls();
    let [ExecPortCall::Exec(target)] = calls.as_slice() else {
        panic!("one exec");
    };
    assert_eq!(target.command, argv);
    assert!(target.tty && target.stdin);

    for empty in [vec![], vec!["  ".to_owned()]] {
        let Err(err) = f.service.exec(&f.pod(), Some("db"), &empty).await else {
            panic!("no command to run");
        };
        assert_eq!(err.kind(), ErrorKind::Validation);
    }
    assert_eq!(f.exec.recorded_calls().len(), 1);
}

#[tokio::test]
async fn a_cluster_that_is_not_open_or_connected_is_reported_and_retryable() {
    let f = Fixture::new();
    let elsewhere = crate::testing::pod("b", "web-0");
    let Err(err) = f.service.attach(&elsewhere, Some("app")).await else {
        panic!("cluster b is not open");
    };
    assert_eq!(err.kind(), ErrorKind::NotFound);

    f.h.manager
        .open(&crate::testing::cluster_context("b"), Default::default());
    let Err(err) = f.service.attach(&elsewhere, Some("app")).await else {
        panic!("cluster b is not connected");
    };
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.is_retryable());
}

#[tokio::test]
async fn permission_and_pod_errors_keep_their_kind_and_name_the_pod() {
    let f = Fixture::new();
    f.exec.script().attach.push_err(OxiError::forbidden(
        "not allowed to attach in pod default/web-0 (needs `create` on `pods/attach`)",
    ));
    let Err(err) = f.service.attach(&f.pod(), Some("app")).await else {
        panic!("forbidden");
    };
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("pods/attach"), "{}", err.message());

    f.exec.script().attach.push_err(OxiError::conflict(
        "pod default/web-0 is terminating and cannot attach",
    ));
    let Err(err) = f.service.attach(&f.pod(), Some("app")).await else {
        panic!("terminating");
    };
    assert!(err.message().contains("terminating"));
    assert!(err.is_retryable());

    f.exec.script().attach.push_err(OxiError::not_found(
        "container nope not found in pod default/web-0",
    ));
    let Err(err) = f.service.attach(&f.pod(), Some("nope")).await else {
        panic!("unknown container");
    };
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn the_notice_cannot_carry_escape_sequences() {
    let line = notice_line_for_tests("ba\x1b[31msh\x07 in web-0/app\r\nINJECTED");
    let text = String::from_utf8(line.to_vec()).unwrap();
    let inner = text
        .strip_prefix("\x1b[2m")
        .and_then(|t| t.strip_suffix("\x1b[0m\r\n"))
        .expect("framed by the app's own dim sequence");
    assert!(
        !inner.contains('\x1b') && !inner.contains('\x07'),
        "{inner:?}"
    );
    assert_eq!(inner.lines().count(), 1, "one line: {inner:?}");
}
