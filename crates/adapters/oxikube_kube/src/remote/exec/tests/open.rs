//! Opening exec and attach against the fake API: what is validated before a request, the
//! request that is sent, and how a refusal maps to the error taxonomy.

use oxikube_domain::ErrorKind;
use oxikube_ports::{ExecOptions, ExecPort};
use serde_json::json;

use crate::fake_api::{FakeApi, status_body};
use crate::remote::exec::KubeExec;

const EXEC: &str = "/api/v1/namespaces/default/pods/p/exec";
const ATTACH: &str = "/api/v1/namespaces/default/pods/p/attach";
const POD: &str = "/api/v1/namespaces/default/pods/p";

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

fn pod(phase: &str, containers: &[&str]) -> serde_json::Value {
    json!({"metadata": {"name": "p", "namespace": "default"},
           "spec": {"containers": containers.iter().map(|c| json!({"name": c, "image": "x"})).collect::<Vec<_>>()},
           "status": {"phase": phase}})
}

async fn exec_error(api: &FakeApi, options: &ExecOptions) -> oxikube_domain::OxiError {
    KubeExec::new(api.client())
        .exec("default", "p", &argv(&["ls"]), options)
        .await
        .expect_err("the open should fail")
}

#[tokio::test]
async fn bad_requests_are_refused_before_any_request() {
    let api = FakeApi::new();
    let exec = KubeExec::new(api.client());
    let tty_stderr = ExecOptions {
        stderr: true,
        ..ExecOptions::interactive()
    };
    let nothing = ExecOptions {
        stdin: false,
        stdout: false,
        stderr: false,
        ..ExecOptions::default()
    };
    for (command, options, pod) in [
        (argv(&["ls"]), tty_stderr, "p"),
        (argv(&["ls"]), nothing, "p"),
        (vec![], ExecOptions::default(), "p"),
        (argv(&[" "]), ExecOptions::default(), "p"),
        (argv(&["ls"]), ExecOptions::default(), "a/b"),
        (argv(&["ls"]), ExecOptions::default().container("c?x"), "p"),
    ] {
        let err = exec
            .exec("default", pod, &command, &options)
            .await
            .expect_err("invalid");
        assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
    }
    let err = exec
        .attach(
            "default",
            "p",
            &ExecOptions {
                tty: true,
                ..ExecOptions::default()
            },
        )
        .await
        .expect_err("a TTY with stderr");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(api.requests().is_empty(), "nothing reached the server");
}

#[tokio::test]
async fn the_request_carries_the_command_and_the_stream_choices() {
    let api = FakeApi::new();
    // Not an upgrade, so the open fails, but the request is recorded.
    api.reply(EXEC, 403, status_body(403, "Forbidden", "no"));
    let options = ExecOptions::interactive().container("app");
    KubeExec::new(api.client())
        .exec("default", "p", &argv(&["sh", "-c", "echo a b"]), &options)
        .await
        .expect_err("refused");
    let request = &api.requests()[0];
    assert_eq!(request.path, EXEC);
    let query = &request.query;
    for expected in [
        "stdin=true",
        "stdout=true",
        "tty=true",
        "container=app",
        "command=sh",
        "command=-c",
        "command=echo+a+b",
    ] {
        assert!(query.contains(expected), "{expected} in {query}");
    }
    assert!(!query.contains("stderr"), "{query}");
}

#[tokio::test]
async fn attach_uses_the_attach_route_without_a_command() {
    let api = FakeApi::new();
    api.reply(ATTACH, 403, status_body(403, "Forbidden", "no"));
    let err = KubeExec::new(api.client())
        .attach("default", "p", &ExecOptions::interactive())
        .await
        .expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("pods/attach"), "{err}");
    assert!(!api.requests()[0].query.contains("command"));
}

#[tokio::test]
async fn refusals_map_to_the_taxonomy() {
    for (code, kind, retryable) in [
        (401, ErrorKind::Auth, true),
        (403, ErrorKind::Forbidden, false),
        (408, ErrorKind::Timeout, true),
        (504, ErrorKind::Timeout, true),
        (500, ErrorKind::Network, true),
    ] {
        let api = FakeApi::new();
        api.reply(EXEC, code, status_body(code, "x", "no"));
        let err = exec_error(&api, &ExecOptions::default()).await;
        assert_eq!(err.kind(), kind, "{code}: {err}");
        assert_eq!(err.is_retryable(), retryable, "{code}: {err}");
    }
}

#[tokio::test]
async fn a_missing_pod_is_not_found() {
    let api = FakeApi::new();
    api.reply(
        EXEC,
        404,
        status_body(404, "NotFound", "pods \"p\" not found"),
    );
    api.reply(
        POD,
        404,
        status_body(404, "NotFound", "pods \"p\" not found"),
    );
    let err = exec_error(&api, &ExecOptions::default()).await;
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("default/p"), "{err}");
}

#[tokio::test]
async fn an_unknown_container_is_not_found() {
    let api = FakeApi::new();
    api.reply(
        EXEC,
        400,
        status_body(400, "BadRequest", "container nope is not valid"),
    );
    api.reply(POD, 200, pod("Running", &["app", "sidecar"]));
    let err = exec_error(&api, &ExecOptions::default().container("nope")).await;
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
    assert!(err.message().contains("container nope"), "{err}");
}

#[tokio::test]
async fn a_pod_that_is_not_running_is_a_conflict() {
    let api = FakeApi::new();
    api.reply(
        EXEC,
        400,
        status_body(400, "BadRequest", "pod is not running"),
    );
    api.reply(POD, 200, pod("Succeeded", &["app"]));
    let err = exec_error(&api, &ExecOptions::default().container("app")).await;
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");
    assert!(err.message().contains("Succeeded"), "{err}");
}

#[tokio::test]
async fn when_the_pod_cannot_be_read_the_first_answer_stands() {
    let api = FakeApi::new();
    api.reply(EXEC, 400, status_body(400, "BadRequest", "bad"));
    api.reply(POD, 403, status_body(403, "Forbidden", "cannot get pods"));
    let err = exec_error(&api, &ExecOptions::default()).await;
    assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
}
