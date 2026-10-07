//! The shell search: bash first, then sh, each probed with a quick exec.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{BackendEvent, ExecTarget, ExitStatus};
use oxikube_testkit::{ExecPortCall, FakeTerminalBackend};
use serde_json::json;

use super::{Fixture, annotated, pod_json, probe_exit};
use crate::exec::{DEFAULT_CONTAINER_ANNOTATION, ShellOptions};

fn exec_targets(f: &Fixture) -> Vec<ExecTarget> {
    f.exec
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            ExecPortCall::Exec(target) => Some(target),
            _ => None,
        })
        .collect()
}

fn programs(f: &Fixture) -> Vec<String> {
    exec_targets(f)
        .iter()
        .map(|t| t.command.join(" "))
        .collect()
}

/// The first line the terminal shows (the notice), as text.
async fn first_line(backend: &dyn oxikube_ports::TerminalBackend) -> String {
    match backend.output_stream().next().await {
        Some(BackendEvent::Output(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        other => panic!("expected the notice first, got {other:?}"),
    }
}

#[tokio::test]
async fn bash_is_probed_and_then_opened_with_a_tty() {
    let f = Fixture::new();
    f.exec.script().exec.push_ok(probe_exit(0));
    let backend = f
        .service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
        .unwrap();

    let targets = exec_targets(&f);
    assert_eq!(targets.len(), 2, "one probe, one session");
    let (probe, session) = (&targets[0], &targets[1]);
    assert_eq!(probe.command, ["bash", "-c", "exit 0"]);
    assert!(!probe.tty && !probe.stdin, "the probe is not interactive");
    assert_eq!(session.command, ["bash"]);
    assert!(session.tty && session.stdin, "the shell is interactive");
    for target in &targets {
        assert_eq!(target.container.as_deref(), Some("app"));
        assert_eq!(target.pod, f.pod());
    }
    let notice = first_line(&*backend).await;
    assert!(notice.contains("bash in web-0/app"), "{notice:?}");
    assert!(
        notice.ends_with("\x1b[0m\r\n"),
        "one line, then the prompt: {notice:?}"
    );
    assert_eq!(
        f.resources.recorded_calls().len(),
        0,
        "a named container needs no pod read"
    );
}

#[tokio::test]
async fn a_container_without_bash_falls_back_to_sh_on_exit_127() {
    let f = Fixture::new();
    f.exec.script().exec.push_ok(probe_exit(127));
    f.exec.script().exec.push_ok(probe_exit(0));
    let backend = f
        .service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
        .unwrap();
    assert_eq!(
        programs(&f),
        ["bash -c exit 0", "sh -c exit 0", "sh"],
        "bash probed and missing, sh probed and opened"
    );
    let notice = first_line(&*backend).await;
    assert!(
        notice.contains("bash not found, using sh in web-0/app"),
        "{notice:?}"
    );
}

#[tokio::test]
async fn exit_126_and_the_runtimes_own_message_also_mean_the_shell_is_missing() {
    let f = Fixture::new();
    f.exec.script().exec.push_ok(probe_exit(126));
    f.exec.script().exec.push_ok(probe_exit(0));
    f.service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
        .unwrap();
    assert_eq!(programs(&f).last().map(String::as_str), Some("sh"));

    // The runtime reports "executable file not found" as a failure without an exit code.
    let g = Fixture::new();
    let no_code = FakeTerminalBackend::silent();
    no_code.exit(ExitStatus {
        code: None,
        message: Some(
            "OCI runtime exec failed: exec failed: unable to start container process: exec: \"bash\": executable file not found in $PATH".into(),
        ),
        ..ExitStatus::default()
    });
    g.exec.script().exec.push_ok(no_code);
    g.exec.script().exec.push_ok(probe_exit(0));
    g.service
        .open_shell(&g.pod(), Some("app"), &ShellOptions::default())
        .await
        .unwrap();
    assert_eq!(programs(&g).last().map(String::as_str), Some("sh"));

    // Or the open itself fails with that text.
    let h = Fixture::new();
    h.exec.script().exec.push_err(OxiError::conflict(
        "exec: \"bash\": executable file not found in $PATH",
    ));
    h.exec.script().exec.push_ok(probe_exit(0));
    h.service
        .open_shell(&h.pod(), Some("app"), &ShellOptions::default())
        .await
        .unwrap();
    assert_eq!(programs(&h).last().map(String::as_str), Some("sh"));
}

#[tokio::test]
async fn no_shell_at_all_points_to_a_debug_container() {
    let f = Fixture::new();
    f.exec.script().exec.push_ok(probe_exit(127));
    f.exec.script().exec.push_ok(probe_exit(127));
    f.serve(pod_json(&[("app", true)]));
    let Err(err) = f
        .service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
    else {
        panic!("a container without a shell must not open one");
    };
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(
        err.message().contains("No shell (bash, sh)"),
        "{}",
        err.message()
    );
    assert!(err.message().contains("container app"), "{}", err.message());
    assert!(
        err.message().contains("debug container"),
        "{}",
        err.message()
    );
    assert!(!err.is_retryable(), "trying again cannot help");
    assert_eq!(programs(&f).len(), 2, "no interactive session was opened");
}

#[tokio::test]
async fn a_windows_pod_gets_windows_advice() {
    let f = Fixture::new();
    f.exec.script().exec.push_ok(probe_exit(127));
    f.exec.script().exec.push_ok(probe_exit(127));
    let mut json = pod_json(&[("app", true)]);
    json["spec"]["os"] = json!({"name": "windows"});
    f.serve(json);
    let Err(err) = f
        .service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
    else {
        panic!("no shell");
    };
    assert!(err.message().contains("Windows"), "{}", err.message());
    assert!(err.message().contains("powershell"), "{}", err.message());
}

#[tokio::test]
async fn the_chain_comes_from_the_settings() {
    let f = Fixture::new();
    f.exec.script().exec.push_ok(probe_exit(127));
    f.exec.script().exec.push_ok(probe_exit(0));
    f.service
        .open_shell(
            &f.pod(),
            Some("app"),
            &ShellOptions::with_shells(["zsh", "/bin/ash"]),
        )
        .await
        .unwrap();
    assert_eq!(
        programs(&f),
        ["zsh -c exit 0", "/bin/ash -c exit 0", "/bin/ash"]
    );
}

#[tokio::test]
async fn a_real_failure_stops_the_search_instead_of_trying_the_next_shell() {
    let f = Fixture::new();
    f.exec.script().exec.push_err(OxiError::forbidden(
        "not allowed to exec in pod default/web-0",
    ));
    let Err(err) = f
        .service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
    else {
        panic!("forbidden must fail");
    };
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(err.message().contains("default/web-0"));
    assert_eq!(programs(&f).len(), 1, "sh was not tried");
}

#[tokio::test]
async fn a_container_that_is_not_running_is_a_retryable_conflict_with_advice() {
    let f = Fixture::new();
    f.exec.script().exec.push_err(OxiError::conflict(
        "the container in pod default/web-0 is not running yet, or has stopped",
    ));
    let Err(err) = f
        .service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
    else {
        panic!("a stopped container must fail");
    };
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.is_retryable(), "the terminal offers retry");
    assert!(err.message().contains("not running"), "{}", err.message());
    assert!(err.message().contains("Open it again"), "{}", err.message());
}

#[tokio::test]
async fn without_a_container_the_pods_default_is_opened() {
    let f = Fixture::new();
    f.serve(annotated(
        pod_json(&[("app", true), ("proxy", true)]),
        DEFAULT_CONTAINER_ANNOTATION,
        "proxy",
    ));
    f.exec.script().exec.push_ok(probe_exit(0));
    f.service
        .open_shell(&f.pod(), None, &ShellOptions::default())
        .await
        .unwrap();
    for target in exec_targets(&f) {
        assert_eq!(target.container.as_deref(), Some("proxy"));
    }
    assert_eq!(
        f.service.last_container(&f.pod()).as_deref(),
        Some("proxy"),
        "the container opened is remembered for the picker"
    );
}

#[tokio::test]
async fn a_pod_that_cannot_be_read_leaves_the_choice_to_the_api_server() {
    let f = Fixture::new();
    f.resources
        .script()
        .get
        .push_err(OxiError::forbidden("pods \"web-0\" is forbidden"));
    f.exec.script().exec.push_ok(probe_exit(0));
    f.service
        .open_shell(&f.pod(), None, &ShellOptions::default())
        .await
        .unwrap();
    for target in exec_targets(&f) {
        assert_eq!(target.container, None, "exec without get still works");
    }
}

#[tokio::test]
async fn a_pod_that_does_not_exist_is_not_found_before_any_exec() {
    let f = Fixture::new();
    f.resources
        .script()
        .get
        .push_err(OxiError::not_found("pods \"web-0\" not found"));
    let Err(err) = f
        .service
        .open_shell(&f.pod(), None, &ShellOptions::default())
        .await
    else {
        panic!("missing pod");
    };
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(exec_targets(&f).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_probe_that_never_answers_times_out() {
    let f = Fixture::new();
    // Silent and never exits: the probe would wait for ever.
    f.exec.script().exec.push_ok(FakeTerminalBackend::silent());
    let options = ShellOptions {
        probe_timeout: Duration::from_secs(3),
        ..ShellOptions::default()
    };
    let Err(err) = f.service.open_shell(&f.pod(), Some("app"), &options).await else {
        panic!("a hung probe must time out");
    };
    assert_eq!(err.kind(), ErrorKind::Timeout);
}

#[tokio::test]
async fn what_the_probe_prints_is_never_kept_or_shown() {
    let f = Fixture::new();
    let chatty = FakeTerminalBackend::silent();
    chatty.output("bash: secret-token-123: command not found\n");
    chatty.exit(ExitStatus::success());
    f.exec.script().exec.push_ok(chatty);
    let backend = f
        .service
        .open_shell(&f.pod(), Some("app"), &ShellOptions::default())
        .await
        .unwrap();
    let mut stream = backend.output_stream();
    let mut seen = String::new();
    // The session is the echo backend: only the notice arrives before anything is typed.
    if let Some(BackendEvent::Output(bytes)) = stream.next().await {
        seen.push_str(&String::from_utf8_lossy(&bytes));
    }
    assert!(!seen.contains("secret-token-123"), "{seen:?}");
}
