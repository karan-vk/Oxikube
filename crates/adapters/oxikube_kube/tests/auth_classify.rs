//! Table tests: recorded error bodies -> `OxiError` kind and `retryable`.
//!
//! Fixtures live under `tests/fixtures/auth/` and use dummy identities and fake tokens only.

use std::io;

use kube::client::AuthError;
use kube::core::Status;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_kube::auth::{CredentialRefresh, classify, classify_with};
use serde::Deserialize;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("fixtures/auth/", $name))
    };
}

fn api_error(body: &str) -> kube::Error {
    let status: Status = serde_json::from_str(body).expect("fixture is a Status");
    kube::Error::Api(Box::new(status))
}

#[derive(Deserialize)]
struct ExecRun {
    exit_code: i32,
    stderr: String,
}

#[derive(Deserialize)]
struct ExecStart {
    io_kind: String,
    os_error: String,
}

fn exit_status(code: i32) -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        std::os::unix::process::ExitStatusExt::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        std::os::windows::process::ExitStatusExt::from_raw(code as u32)
    }
}

fn exec_run_error(body: &str) -> kube::Error {
    let r: ExecRun = serde_json::from_str(body).expect("fixture is an exec run");
    kube::Error::Auth(AuthError::AuthExecRun {
        // A realistic std `Command` Debug: env first, then the program and its arguments.
        cmd: r#"KUBERNETES_EXEC_INFO="{}" AWS_SECRET_ACCESS_KEY="FAKE-SECRET-KEY" "aws" "eks" "get-token""#.into(),
        status: exit_status(r.exit_code),
        out: std::process::Output {
            status: exit_status(r.exit_code),
            stdout: b"{\"status\":{\"token\":\"FAKE-STDOUT-TOKEN\"}}".to_vec(),
            stderr: r.stderr.into_bytes(),
        },
    })
}

fn assert_class(err: &OxiError, kind: ErrorKind, retryable: bool) {
    assert_eq!(err.kind(), kind, "{err:?}");
    assert_eq!(err.is_retryable(), retryable, "{err:?}");
}

#[test]
fn api_status_fixtures_map_to_kinds() {
    let table = [
        (fixture!("status_401.json"), ErrorKind::Auth, true),
        (fixture!("status_403.json"), ErrorKind::Forbidden, false),
        (fixture!("status_404.json"), ErrorKind::NotFound, false),
        (fixture!("status_409.json"), ErrorKind::Conflict, false),
        (fixture!("status_422.json"), ErrorKind::Validation, false),
        (fixture!("status_503.json"), ErrorKind::Network, true),
    ];
    for (body, kind, retryable) in table {
        assert_class(&classify(&api_error(body)), kind, retryable);
    }
}

#[test]
fn a_401_is_retryable_only_when_the_credential_can_refresh() {
    let err = api_error(fixture!("status_401.json"));
    assert_class(
        &classify_with(&err, CredentialRefresh::Refreshable),
        ErrorKind::Auth,
        true,
    );
    assert_class(
        &classify_with(&err, CredentialRefresh::Unknown),
        ErrorKind::Auth,
        true,
    );
    assert_class(
        &classify_with(&err, CredentialRefresh::Static),
        ErrorKind::Auth,
        false,
    );
}

#[test]
fn credential_refresh_follows_the_kubeconfig_user() {
    use kube::config::{AuthInfo, ExecConfig};
    let exec = AuthInfo {
        exec: Some(ExecConfig::default()),
        ..Default::default()
    };
    let token_file = AuthInfo {
        token_file: Some("/var/run/t".into()),
        ..Default::default()
    };
    let token = AuthInfo {
        token: Some("FAKE".to_string().into()),
        ..Default::default()
    };
    assert_eq!(CredentialRefresh::of(&exec), CredentialRefresh::Refreshable);
    assert_eq!(
        CredentialRefresh::of(&token_file),
        CredentialRefresh::Refreshable
    );
    assert_eq!(CredentialRefresh::of(&token), CredentialRefresh::Static);
    assert_eq!(
        CredentialRefresh::of(&AuthInfo::default()),
        CredentialRefresh::Static
    );
}

#[test]
fn forbidden_message_names_user_resource_and_verb_but_no_credential() {
    let err = classify(&api_error(fixture!("status_403.json")));
    let msg = err.message();
    assert!(msg.contains("cannot list resource \"pods\""), "{msg}");
    assert!(msg.contains("namespace \"demo\""), "{msg}");
    assert!(msg.contains("system:serviceaccount:demo:viewer"), "{msg}");
}

#[test]
fn expired_token_body_is_redacted_and_still_auth() {
    let err = classify(&api_error(fixture!("status_401_expired_token.json")));
    assert_class(&err, ErrorKind::Auth, true);
    assert!(
        err.message().contains("token has expired"),
        "{}",
        err.message()
    );
    assert_no_leak(&err, "FAKE.EXPIRED.TOKEN");
}

#[test]
fn exec_not_found_is_a_permanent_auth_error() {
    let f: ExecStart = serde_json::from_str(fixture!("exec_not_found.json")).unwrap();
    assert_eq!(f.io_kind, "NotFound");
    let err = classify(&kube::Error::Auth(AuthError::AuthExecStart(
        io::Error::new(io::ErrorKind::NotFound, f.os_error),
    )));
    assert_class(&err, ErrorKind::Auth, false);
    assert!(err.message().contains("not found"), "{}", err.message());
}

#[test]
fn exec_non_zero_exit_is_retryable_and_never_leaks() {
    let err = classify(&exec_run_error(fixture!("exec_nonzero_exit.json")));
    assert_class(&err, ErrorKind::Auth, true);
    assert!(
        err.message().contains("credentials service unavailable"),
        "{}",
        err.message()
    );
    for secret in [
        "FAKE.LEAK.TOKEN",
        "FAKE-SECRET-KEY",
        "FAKE-STDOUT-TOKEN",
        "KUBERNETES_EXEC_INFO",
        "get-token",
    ] {
        assert_no_leak(&err, secret);
    }
}

#[test]
fn plugin_network_failure_mentioning_login_is_still_retryable() {
    let err = classify(&exec_run_error(fixture!("exec_network_timeout.json")));
    assert_class(&err, ErrorKind::Auth, true);
    assert!(
        !err.message().contains("sign in or answer"),
        "{}",
        err.message()
    );
}

#[test]
fn exec_waiting_for_a_prompt_is_not_retryable_and_says_why() {
    let err = classify(&exec_run_error(fixture!("exec_mfa_prompt.json")));
    assert_class(&err, ErrorKind::Auth, false);
    assert!(
        err.message().contains("sign in or answer a prompt"),
        "{}",
        err.message()
    );
    assert!(
        err.message().contains("Enter MFA code"),
        "{}",
        err.message()
    );
}

#[test]
fn token_file_and_oidc_failures_are_auth() {
    let missing = AuthError::ReadTokenFile(
        io::Error::from(io::ErrorKind::NotFound),
        "/var/run/token".into(),
    );
    assert_class(
        &classify(&kube::Error::Auth(missing)),
        ErrorKind::Auth,
        false,
    );
    assert_class(
        &classify(&kube::Error::Auth(AuthError::ExecPluginFailed)),
        ErrorKind::Auth,
        true,
    );
}

#[test]
fn transport_failures_are_network_or_timeout() {
    let refused = kube::Error::Service(Box::new(io::Error::from(io::ErrorKind::ConnectionRefused)));
    assert_class(&classify(&refused), ErrorKind::Network, true);
    let timed_out = kube::Error::Service(Box::new(io::Error::from(io::ErrorKind::TimedOut)));
    assert_class(&classify(&timed_out), ErrorKind::Timeout, true);
    let nested = kube::Error::Service(Box::new(io::Error::other(io::Error::from(
        io::ErrorKind::ConnectionReset,
    ))));
    assert_class(&classify(&nested), ErrorKind::Network, true);
}

#[test]
fn an_auth_error_wrapped_in_a_service_error_is_still_auth() {
    let wrapped = kube::Error::Service(Box::new(AuthError::MissingCommand));
    assert_class(&classify(&wrapped), ErrorKind::Auth, false);
    let wrapped_kube = kube::Error::Service(Box::new(api_error(fixture!("status_401.json"))));
    assert_class(&classify(&wrapped_kube), ErrorKind::Auth, true);
}

#[test]
fn unclassified_statuses_are_internal_or_unsupported() {
    let teapot = Status::failure("short and stout", "").with_code(418);
    assert_eq!(
        classify(&kube::Error::Api(Box::new(teapot))).kind(),
        ErrorKind::Internal
    );
    let not_impl = Status::failure("not implemented", "").with_code(501);
    assert_eq!(
        classify(&kube::Error::Api(Box::new(not_impl))).kind(),
        ErrorKind::Unsupported
    );
    let gone = Status::failure("too old resource version", "Expired").with_code(410);
    assert_eq!(
        classify(&kube::Error::Api(Box::new(gone))).kind(),
        ErrorKind::Conflict
    );
}

fn assert_no_leak(err: &OxiError, secret: &str) {
    assert!(
        !err.message().contains(secret),
        "message leaks {secret}: {}",
        err.message()
    );
    assert!(!err.to_string().contains(secret), "Display leaks {secret}");
    assert!(
        !format!("{err:?}").contains(secret),
        "Debug leaks {secret}: {err:?}"
    );
}
