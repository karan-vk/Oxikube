//! Epic E17 "done when" (4) against kind: the kubeconfig `serviceaccount::GenerateKubeconfig`
//! builds works with `kubectl` itself (it authenticates as the account and is held to its Role),
//! and its token expires when asked: the default lifetime or the one given, as the server
//! reports it (`TokenRequest` takes the asked lifetime as is on kind; a server with
//! `--service-account-max-token-expiration` may shorten it, which `expires_at` would show).
//!
//! The file `kubectl` reads is the only kubeconfig written to disk in the suite: owner-only, in a
//! temp dir deleted when the test ends. Nothing prints the token.

use std::path::Path;
use std::process::Command as Process;

use jiff::Timestamp;
use oxikube_app::sa_kubeconfig::{DEFAULT_EXPIRATION_SECONDS, GeneratedKubeconfig};
use oxikube_testkit::integration::rbac::AppliedScenario;

use super::Admin;
use crate::cluster::Kind;

/// How far apart the server's clock and ours, and the request's round trip, may put the expiry.
const SLACK_SECONDS: i64 = 60;

/// Writes the kubeconfig for `kubectl`, readable by the owner only.
fn write_private(path: &Path, generated: &GeneratedKubeconfig) {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut file| file.write_all(generated.text.expose().as_bytes()))
        .expect("write the kubeconfig");
}

/// `kubectl --kubeconfig <path> <args>`: (success, stdout, stderr).
fn kubectl(path: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Process::new("kubectl")
        .arg("--kubeconfig")
        .arg(path)
        .args(args)
        .output()
        .expect("kubectl runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    )
}

/// Seconds from `since` to the expiry the server reported.
fn lifetime(generated: &GeneratedKubeconfig, since: Timestamp) -> i64 {
    let expires: Timestamp = generated
        .expires_at
        .as_deref()
        .expect("a TokenRequest says when the token expires")
        .parse()
        .expect("an RFC 3339 time");
    expires.as_second() - since.as_second()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_generated_kubeconfig_works_with_kubectl_and_expires_when_asked() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let applied = AppliedScenario::apply(kind.context.as_str()).expect("the RBAC scenario");
    let scenario = applied.scenario().clone();
    let admin = Admin::connect(&kind).await;

    // --- the expiry: the default lifetime, then an asked one ----------------------------------
    for asked in [None, Some(1200)] {
        let since = Timestamp::now();
        let generated = admin.generate_kubeconfig(&applied, asked).await;
        let expected = i64::from(asked.unwrap_or(DEFAULT_EXPIRATION_SECONDS));
        let actual = lifetime(&generated, since);
        assert!(
            (expected - SLACK_SECONDS..=expected + SLACK_SECONDS).contains(&actual),
            "asked {asked:?}: the token lives {actual} s, expected about {expected} s"
        );
    }

    // --- kubectl with the file -----------------------------------------------------------------
    let generated = admin.generate_kubeconfig(&applied, None).await;
    let token = generated
        .text
        .expose()
        .lines()
        .find_map(|line| line.trim().strip_prefix("token: "))
        .expect("a token line")
        .to_owned();
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("limited.kubeconfig");
    write_private(&path, &generated);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only");
    }

    let (ok, who, stderr) = kubectl(
        &path,
        &[
            "auth",
            "whoami",
            "-o",
            "jsonpath={.status.userInfo.username}",
        ],
    );
    assert!(ok, "kubectl auth whoami: {stderr}");
    assert_eq!(who, scenario.username());
    let (ok, _, stderr) = kubectl(&path, &["get", "pods", "-n", &scenario.home]);
    assert!(ok, "the Role lets the account list pods at home: {stderr}");
    for args in [
        ["get", "secrets", "-n", scenario.home.as_str()],
        ["get", "pods", "-n", scenario.other.as_str()],
    ] {
        let (ok, _, stderr) = kubectl(&path, &args);
        assert!(!ok, "{args:?} must be refused");
        assert!(stderr.contains("forbidden"), "{args:?}: {stderr}");
        assert!(!stderr.contains(&token), "kubectl never echoes the token");
    }
    drop(dir);
    assert!(!path.exists(), "the kubeconfig is gone with the temp dir");
}
