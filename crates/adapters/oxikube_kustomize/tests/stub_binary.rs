//! The adapter against stub executables on a controlled `PATH` (no kustomize needed).
#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_domain::kustomize::KustomizeToolKind;
use oxikube_kustomize::{KustomizeCli, KustomizeConfig, SearchPath};
use oxikube_ports::KustomizePort;
use tempfile::TempDir;

/// Writes an executable `sh` script called `name` into `dir`.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn cli_in(dir: &Path) -> KustomizeCli {
    KustomizeCli::new(KustomizeConfig {
        search_path: SearchPath::Fixed(OsString::from(dir)),
        build_timeout: Duration::from_secs(10),
        probe_timeout: Duration::from_secs(5),
        ..KustomizeConfig::default()
    })
}

const KUSTOMIZE_OK: &str = r#"
case "$1" in
  version) echo v5.4.2 ;;
  build) echo "kind: ConfigMap"; echo "warning: deprecated" >&2 ;;
esac
"#;

fn kubectl_stub(dir: &Path, has_kustomize: bool) {
    let help = if has_kustomize { 0 } else { 1 };
    let body = format!(
        r#"
case "$1" in
  kustomize) if [ "$2" = "--help" ]; then exit {help}; fi; echo "kind: Namespace" ;;
  version) echo "Client Version: v1.30.0"; echo "Kustomize Version: v5.0.4" ;;
esac
"#
    );
    script(dir, "kubectl", &body);
}

#[tokio::test]
async fn standalone_kustomize_is_preferred_and_builds() {
    let dir = TempDir::new().expect("tempdir");
    script(dir.path(), "kustomize", KUSTOMIZE_OK);
    kubectl_stub(dir.path(), true);
    let cli = cli_in(dir.path());
    let tool = cli.detect().await.expect("a tool");
    assert_eq!(tool.kind, KustomizeToolKind::Standalone);
    assert_eq!(tool.version, "v5.4.2");
    let out = cli.build(Path::new("/overlay")).await.expect("build");
    assert_eq!(out.yaml.trim(), "kind: ConfigMap");
    assert_eq!(
        out.stderr, "warning: deprecated",
        "stderr stays separate from stdout"
    );
}

#[tokio::test]
async fn kubectl_kustomize_is_the_fallback() {
    let dir = TempDir::new().expect("tempdir");
    kubectl_stub(dir.path(), true);
    let cli = cli_in(dir.path());
    let tool = cli.detect().await.expect("a tool");
    assert_eq!(tool.kind, KustomizeToolKind::Kubectl);
    assert_eq!(tool.version, "v5.0.4");
    assert_eq!(
        cli.build(Path::new("/o")).await.expect("build").yaml.trim(),
        "kind: Namespace"
    );
}

#[tokio::test]
async fn nothing_found_means_no_tool_and_unsupported_builds() {
    let empty = TempDir::new().expect("tempdir");
    let cli = cli_in(empty.path());
    assert_eq!(cli.detect().await, None);
    let error = cli.build(Path::new("/o")).await.expect_err("no tool");
    assert_eq!(error.kind(), ErrorKind::Unsupported);

    let broken = TempDir::new().expect("tempdir");
    kubectl_stub(broken.path(), false);
    assert_eq!(
        cli_in(broken.path()).detect().await,
        None,
        "a kubectl without the kustomize subcommand does not count"
    );
}

#[tokio::test]
async fn a_broken_standalone_falls_through_to_kubectl() {
    let dir = TempDir::new().expect("tempdir");
    script(dir.path(), "kustomize", "exit 3");
    kubectl_stub(dir.path(), true);
    let tool = cli_in(dir.path()).detect().await.expect("a tool");
    assert_eq!(tool.kind, KustomizeToolKind::Kubectl);
}

#[tokio::test]
async fn the_path_override_wins_and_is_never_searched_around() {
    let dir = TempDir::new().expect("tempdir");
    script(dir.path(), "kustomize", KUSTOMIZE_OK);
    let other = TempDir::new().expect("tempdir");
    let mine = script(other.path(), "my-kustomize", "echo v9.9.9");
    let mut config = KustomizeConfig {
        search_path: SearchPath::Fixed(OsString::from(dir.path())),
        path: Some(mine),
        ..KustomizeConfig::default()
    };
    let cli = KustomizeCli::new(config.clone());
    assert_eq!(cli.detect().await.expect("tool").version, "v9.9.9");
    config.path = Some(dir.path().join("missing"));
    cli.reconfigure(config).await;
    assert_eq!(
        cli.detect().await,
        None,
        "a bad override is not replaced by PATH"
    );
}

#[tokio::test]
async fn refresh_notices_a_new_install() {
    let dir = TempDir::new().expect("tempdir");
    let cli = cli_in(dir.path());
    assert_eq!(cli.detect().await, None);
    script(dir.path(), "kustomize", KUSTOMIZE_OK);
    assert_eq!(cli.detect().await, None, "detection is cached");
    assert!(cli.refresh().await.is_some());
}

#[tokio::test]
async fn a_failed_build_carries_stderr_and_the_safe_flags() {
    let dir = TempDir::new().expect("tempdir");
    let args = dir.path().join("args");
    let body = format!(
        r#"
case "$1" in
  version) echo v5.4.2 ;;
  *) echo "$@" > {}; echo "Error: accumulating resources: no such file" >&2; exit 1 ;;
esac
"#,
        args.display()
    );
    script(dir.path(), "kustomize", &body);
    let error = cli_in(dir.path())
        .build(Path::new("/o/prod"))
        .await
        .expect_err("fails");
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        error.message().contains("accumulating resources"),
        "{}",
        error.message()
    );
    let line = std::fs::read_to_string(args).expect("recorded args");
    assert!(!line.contains("--enable"), "{line}");
    assert!(
        line.contains("--load-restrictor=LoadRestrictionsRootOnly"),
        "{line}"
    );
    assert!(line.trim().ends_with("-- /o/prod"), "{line}");
}

#[tokio::test]
async fn a_disabled_helm_or_plugin_feature_is_explained() {
    for (stderr, flag) in [
        ("Error: must specify --enable-helm", "--enable-helm"),
        (
            "Error: unable to find plugin root - tried: x",
            "--enable-alpha-plugins",
        ),
    ] {
        let dir = TempDir::new().expect("tempdir");
        let body = format!(
            r#"case "$1" in version) echo v5.4.2 ;; *) echo "{stderr}" >&2; exit 1 ;; esac"#
        );
        script(dir.path(), "kustomize", &body);
        let error = cli_in(dir.path())
            .build(Path::new("/o"))
            .await
            .expect_err("fails");
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        let message = error.message();
        assert!(message.contains("terminal"), "{message}");
        assert!(message.contains(flag), "{message}");
    }
}

#[tokio::test]
async fn a_build_past_the_deadline_times_out() {
    let dir = TempDir::new().expect("tempdir");
    script(
        dir.path(),
        "kustomize",
        r#"case "$1" in version) echo v5 ;; *) exec /bin/sleep 30 ;; esac"#,
    );
    let cli = KustomizeCli::new(KustomizeConfig {
        search_path: SearchPath::Fixed(OsString::from(dir.path())),
        build_timeout: Duration::from_millis(300),
        ..KustomizeConfig::default()
    });
    let error = cli.build(Path::new("/o")).await.expect_err("times out");
    assert_eq!(error.kind(), ErrorKind::Timeout, "{}", error.message());
}

/// Whether process `pid` still exists.
fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[tokio::test]
async fn dropping_the_build_kills_the_child() {
    let dir = TempDir::new().expect("tempdir");
    let pidfile = dir.path().join("pid");
    let body = format!(
        r#"case "$1" in version) echo v5 ;; *) echo $$ > {}; exec /bin/sleep 30 ;; esac"#,
        pidfile.display()
    );
    script(dir.path(), "kustomize", &body);
    let cli = cli_in(dir.path());
    // Cancelling is dropping the future.
    let cancelled =
        tokio::time::timeout(Duration::from_millis(500), cli.build(Path::new("/o"))).await;
    assert!(
        cancelled.is_err(),
        "the build is still running when dropped"
    );
    let pid = std::fs::read_to_string(&pidfile)
        .expect("pid file")
        .trim()
        .to_owned();
    let mut waited = 0;
    while alive(&pid) && waited < 50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        waited += 1;
    }
    assert!(!alive(&pid), "child {pid} outlived the dropped build");
}

#[tokio::test]
async fn ten_megabytes_of_output_do_not_stall_the_child() {
    let dir = TempDir::new().expect("tempdir");
    script(
        dir.path(),
        "kustomize",
        r#"case "$1" in version) echo v5 ;; *) /usr/bin/head -c 10485760 /dev/zero | /usr/bin/tr '\0' 'a'; echo "w" >&2 ;; esac"#,
    );
    let out = cli_in(dir.path())
        .build(Path::new("/o"))
        .await
        .expect("build");
    assert_eq!(out.yaml.len(), 10 * 1024 * 1024);
    assert_eq!(out.stderr, "w");
}

/// Runs the real binary when one is installed; skipped (with a message) otherwise.
#[tokio::test]
async fn a_real_kustomize_builds_a_small_overlay() {
    let cli = KustomizeCli::new(KustomizeConfig::default());
    let Some(tool) = cli.detect().await else {
        eprintln!("skipped: neither kustomize nor kubectl kustomize is installed");
        return;
    };
    let dir = TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("kustomization.yaml"),
        "resources:\n  - cm.yaml\nnamePrefix: demo-\n",
    )
    .expect("kustomization");
    std::fs::write(
        dir.path().join("cm.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: app\ndata:\n  k: v\n",
    )
    .expect("cm");
    let out = cli
        .build(dir.path())
        .await
        .expect("build with the real tool");
    assert!(out.yaml.contains("name: demo-app"), "{tool}: {}", out.yaml);
}
