//! `CliRunner` against script-based fake CLIs in a temp dir (E18-S01): success with JSON flags,
//! non-zero exit with redacted stderr, timeout, cancellation killing the child, invalid JSON,
//! a missing binary, the concurrency cap, and the login shell `PATH` probe with its fallbacks.
//! No real cloud CLI, account or network is involved.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_cloud::process::{
    Cli, CliError, CliInvocation, CliRunner, CliRunnerConfig, LoginShell, ProcessRunner, RunOpts,
    login_shell_path,
};
use oxikube_domain::{ErrorKind, OxiError};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Writes an executable `/bin/sh` script `name` with `body` into `dir`.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("the temp dir is writable");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("the script was just written");
    path
}

/// A runner whose `aws` is `aws_body`, with the login shell probe off.
fn runner_with_aws(dir: &TempDir, aws_body: &str) -> Arc<dyn ProcessRunner> {
    let aws = script(dir.path(), "aws", aws_body);
    let config = CliRunnerConfig {
        paths: BTreeMap::from([(Cli::Aws, aws)]),
        login_shell: LoginShell::Disabled,
        ..CliRunnerConfig::default()
    };
    Arc::new(CliRunner::new(config))
}

fn opts(timeout: Duration) -> RunOpts {
    RunOpts {
        timeout: Some(timeout),
        ..RunOpts::default()
    }
}

/// `true` once `pid` is gone (or a zombie: killed, waiting to be reaped).
fn is_dead(pid: &str) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .expect("ps runs on every Unix");
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    stat.is_empty() || stat.starts_with('Z')
}

#[tokio::test]
async fn success_appends_the_json_flags_and_passes_the_environment() {
    let dir = TempDir::new().unwrap();
    let body = r#"printf '{"args":"%s","profile":"%s","pager":"%s"}' "$*" "$AWS_PROFILE" "${AWS_PAGER-unset}""#;
    let runner = runner_with_aws(&dir, body);
    let invocation = CliInvocation::new(Cli::Aws, ["eks", "list-clusters"]).with_opts(RunOpts {
        env: vec![("AWS_PROFILE".into(), "prod".into())],
        ..RunOpts::default()
    });
    let out: serde_json::Value = runner.run_json(invocation).await.unwrap();
    assert_eq!(out["args"], "eks list-clusters --output json");
    assert_eq!(out["profile"], "prod");
    assert_eq!(out["pager"], "", "the pager is switched off");

    let text = runner
        .run(CliInvocation::new(Cli::Aws, ["--version"]))
        .await
        .unwrap();
    assert!(
        text.stdout.contains(r#""args":"--version""#),
        "{}",
        text.stdout
    );
}

#[tokio::test]
async fn stdin_is_closed_so_a_reading_cli_does_not_hang() {
    let dir = TempDir::new().unwrap();
    let runner = runner_with_aws(&dir, "cat; echo done");
    let started = Instant::now();
    let out = runner
        .run(CliInvocation::new(Cli::Aws, ["configure"]).with_opts(opts(Duration::from_secs(10))))
        .await
        .unwrap();
    assert_eq!(out.stdout.trim(), "done");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_non_zero_exit_is_a_typed_error_with_redacted_stderr() {
    let dir = TempDir::new().unwrap();
    let jwt = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjEifQ.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";
    let body = format!(
        "echo 'An error occurred (AccessDeniedException) when calling the ListClusters operation: token {jwt}' >&2\nexit 254"
    );
    let runner = runner_with_aws(&dir, &body);
    let error = runner
        .run(CliInvocation::new(
            Cli::Aws,
            ["eks", "list-clusters", "--region", "eu-west-1"],
        ))
        .await
        .unwrap_err();
    let CliError::Exit {
        code,
        ref stderr,
        ref command,
        ..
    } = error
    else {
        panic!("expected an exit error, got {error:?}");
    };
    assert_eq!(code, Some(254));
    assert_eq!(
        command, "aws eks list-clusters",
        "flag values stay out of the label"
    );
    assert!(stderr.contains("AccessDeniedException"));
    assert!(!stderr.contains(jwt), "stderr is redacted: {stderr}");
    let mapped = OxiError::from(error);
    assert_eq!(mapped.kind(), ErrorKind::Forbidden);
    assert!(!mapped.message().contains(jwt));
}

#[tokio::test]
async fn a_hung_cli_is_killed_at_its_timeout() {
    let dir = TempDir::new().unwrap();
    let pid_file = dir.path().join("pid");
    let body = format!("echo $$ > '{}'\nexec sleep 30", pid_file.display());
    let runner = runner_with_aws(&dir, &body);
    let started = Instant::now();
    let error = runner
        .run(CliInvocation::new(Cli::Aws, ["eks"]).with_opts(opts(Duration::from_secs(1))))
        .await
        .unwrap_err();
    assert!(matches!(error, CliError::Timeout { .. }), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(OxiError::from(error).kind(), ErrorKind::Timeout);
    // Under heavy load the timeout can fire before the script wrote its pid; the process was
    // killed all the same, there is just nothing left to look up.
    if let Ok(pid) = std::fs::read_to_string(&pid_file) {
        wait_dead(pid.trim()).await;
    }
}

#[tokio::test]
async fn cancelling_kills_the_child() {
    let dir = TempDir::new().unwrap();
    let pid_file = dir.path().join("pid");
    let body = format!("echo $$ > '{}'\nexec sleep 30", pid_file.display());
    let runner = runner_with_aws(&dir, &body);
    let cancel = CancellationToken::new();
    let invocation = CliInvocation::new(Cli::Aws, ["eks"]).with_opts(RunOpts {
        cancel: cancel.clone(),
        ..opts(Duration::from_secs(60))
    });
    let task = tokio::spawn({
        let runner = Arc::clone(&runner);
        async move { runner.run(invocation).await }
    });
    let pid = wait_for_file(&pid_file).await;
    assert!(!is_dead(&pid), "the child is running before the cancel");
    cancel.cancel();
    let error = task.await.unwrap().unwrap_err();
    assert!(matches!(error, CliError::Cancelled { .. }), "{error:?}");
    wait_dead(&pid).await;

    let already = CancellationToken::new();
    already.cancel();
    let error = runner
        .run(CliInvocation::new(Cli::Aws, ["eks"]).with_opts(RunOpts {
            cancel: already,
            ..RunOpts::default()
        }))
        .await
        .unwrap_err();
    assert!(matches!(error, CliError::Cancelled { .. }), "{error:?}");
}

#[tokio::test]
async fn output_that_is_not_json_is_an_invalid_json_error() {
    let dir = TempDir::new().unwrap();
    let runner = runner_with_aws(&dir, "echo 'clusters: prod'");
    let error = runner
        .run_json::<serde_json::Value>(CliInvocation::new(Cli::Aws, ["eks", "list-clusters"]))
        .await
        .unwrap_err();
    assert!(matches!(error, CliError::InvalidJson { .. }), "{error:?}");
    assert_eq!(OxiError::from(error).kind(), ErrorKind::Internal);
}

#[tokio::test]
async fn a_missing_binary_is_unsupported_naming_its_setting() {
    let dir = TempDir::new().unwrap();
    let config = CliRunnerConfig {
        paths: BTreeMap::from([(Cli::Gcloud, dir.path().join("nope"))]),
        login_shell: LoginShell::Disabled,
        ..CliRunnerConfig::default()
    };
    let runner: Arc<dyn ProcessRunner> = Arc::new(CliRunner::new(config));
    assert_eq!(runner.locate(Cli::Gcloud).await, None);
    let error = runner
        .run(CliInvocation::new(Cli::Gcloud, ["info"]))
        .await
        .unwrap_err();
    assert_eq!(error, CliError::NotFound { cli: Cli::Gcloud });
    let mapped = OxiError::from(error);
    assert_eq!(mapped.kind(), ErrorKind::Unsupported);
    assert!(mapped.message().contains("cloud.gcloud_path"));
}

#[tokio::test]
async fn the_concurrency_cap_queues_extra_runs() {
    let dir = TempDir::new().unwrap();
    let aws = script(dir.path(), "aws", "sleep 0.3");
    let config = CliRunnerConfig {
        paths: BTreeMap::from([(Cli::Aws, aws)]),
        login_shell: LoginShell::Disabled,
        max_concurrent: 1,
        ..CliRunnerConfig::default()
    };
    let runner: Arc<dyn ProcessRunner> = Arc::new(CliRunner::new(config));
    let started = Instant::now();
    let run = || runner.run(CliInvocation::new(Cli::Aws, ["eks"]));
    let (a, b) = tokio::join!(run(), run());
    a.unwrap();
    b.unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(550),
        "two runs overlapped under a cap of 1: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn the_login_shell_path_finds_a_cli_the_app_path_lacks() {
    let tools = TempDir::new().unwrap();
    script(tools.path(), "gcloud", "echo gcloud-from-login-path");
    let shell_dir = TempDir::new().unwrap();
    // A stand-in login shell: ignores `-l -c <script>` and prints what the probe expects,
    // after some profile noise.
    let shell = script(
        shell_dir.path(),
        "fake-shell",
        &format!(
            "echo 'Welcome back'\necho __OXIKUBE_PATH_START__\necho '{}:/usr/bin:/bin'\necho __OXIKUBE_PATH_END__",
            tools.path().display()
        ),
    );
    let config = CliRunnerConfig {
        login_shell: LoginShell::Program(shell),
        ..CliRunnerConfig::default()
    };
    let runner: Arc<dyn ProcessRunner> = Arc::new(CliRunner::new(config));
    let found = runner.locate(Cli::Gcloud).await.unwrap();
    assert_eq!(found, tools.path().join("gcloud"));
    let out = runner
        .run(CliInvocation::new(Cli::Gcloud, ["version"]))
        .await
        .unwrap();
    assert_eq!(out.stdout.trim(), "gcloud-from-login-path");
}

#[tokio::test]
async fn a_failing_or_hanging_login_shell_falls_back_cleanly() {
    let dir = TempDir::new().unwrap();
    let failing = script(dir.path(), "failing-shell", "echo oops >&2\nexit 1");
    let hanging = script(dir.path(), "hanging-shell", "exec sleep 30");
    let noisy = script(dir.path(), "noisy-shell", "echo '/not/a/marked/path'");
    let timeout = Duration::from_millis(300);
    assert_eq!(login_shell_path(&failing, timeout).await, None);
    assert_eq!(login_shell_path(&noisy, timeout).await, None);
    let started = Instant::now();
    assert_eq!(login_shell_path(&hanging, timeout).await, None);
    assert!(started.elapsed() < Duration::from_secs(5), "the probe hung");
    assert_eq!(
        login_shell_path(&dir.path().join("no-such-shell"), timeout).await,
        None
    );

    // A runner whose probe hangs still runs an overridden CLI, after the probe's timeout.
    let aws = script(dir.path(), "aws", "echo ok");
    let config = CliRunnerConfig {
        paths: BTreeMap::from([(Cli::Aws, aws)]),
        login_shell: LoginShell::Program(hanging),
        probe_timeout: timeout,
        ..CliRunnerConfig::default()
    };
    let runner: Arc<dyn ProcessRunner> = Arc::new(CliRunner::new(config));
    let out = runner
        .run(CliInvocation::new(Cli::Aws, ["eks"]))
        .await
        .unwrap();
    assert_eq!(out.stdout.trim(), "ok");
}

#[tokio::test]
async fn the_real_login_shell_reports_a_path_when_there_is_one() {
    // `$SHELL` is whatever the developer or CI uses (zsh, bash, fish); skip when unset.
    let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) else {
        return;
    };
    let path = login_shell_path(Path::new(&shell), Duration::from_secs(10)).await;
    if let Some(path) = path {
        assert!(std::env::split_paths(&path).any(|dir| dir.is_absolute()));
    }
}

async fn wait_for_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && !text.trim().is_empty()
        {
            return text.trim().to_owned();
        }
        assert!(Instant::now() < deadline, "the fake CLI never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_dead(pid: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_dead(pid) {
        assert!(Instant::now() < deadline, "process {pid} survived");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
