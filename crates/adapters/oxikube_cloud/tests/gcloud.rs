//! `GcloudDiscovery` through the real `CliRunner` against fake `gcloud` and
//! `gke-gcloud-auth-plugin` scripts (E18-S03): the whole path from process to kubeconfig
//! entry without a Google account.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use oxikube_cloud::gcloud::GcloudDiscovery;
use oxikube_cloud::process::{Cli, CliRunner, CliRunnerConfig, LoginShell};
use oxikube_ports::cloud::{CloudDiscoveryPort, CloudScope, CloudToolStatus};
use tempfile::TempDir;

fn script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("temp dir writable");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("script just written");
    path
}

const FAKE_GCLOUD: &str = r#"
case "$*" in
  "version --format=json") echo '{"Google Cloud SDK": "999.0.0"}' ;;
  "auth list --format=json") echo '[{"account":"a@example.com","status":"ACTIVE"}]' ;;
  "container clusters list --project p1 --format=json")
    echo '[{"name":"c1","location":"europe-west1-b","endpoint":"198.51.100.7","status":"RUNNING","masterAuth":{"clusterCaCertificate":"QQ=="}}]' ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#;

fn adapter(dir: &TempDir, with_plugin: bool) -> GcloudDiscovery {
    let mut paths = BTreeMap::from([(Cli::Gcloud, script(dir.path(), "gcloud", FAKE_GCLOUD))]);
    // Without the plugin the path points at a file that does not exist, so a plugin
    // installed on the host cannot be found through PATH.
    let plugin = if with_plugin {
        script(dir.path(), "gke-gcloud-auth-plugin", "exit 0")
    } else {
        dir.path().join("absent-gke-gcloud-auth-plugin")
    };
    paths.insert(Cli::GkeGcloudAuthPlugin, plugin);
    let config = CliRunnerConfig {
        paths,
        login_shell: LoginShell::Disabled,
        ..CliRunnerConfig::default()
    };
    GcloudDiscovery::new(Arc::new(CliRunner::new(config)))
}

#[tokio::test]
async fn lists_and_builds_an_entry_through_real_processes() {
    let dir = TempDir::new().expect("temp dir");
    let gke = adapter(&dir, true);
    let detection = gke.detect().await.expect("detect");
    assert_eq!(detection.status, CloudToolStatus::Ready);
    assert_eq!(detection.version.as_deref(), Some("999.0.0"));

    let scope = CloudScope::Gcp {
        project: "p1".into(),
    };
    let clusters = gke.list_clusters(&scope).await.expect("list");
    assert_eq!(clusters.len(), 1);
    let entry = gke.kubeconfig_entry(&clusters[0]).await.expect("entry");
    assert_eq!(entry.context.as_str(), "gke:p1:europe-west1-b:c1");
    assert_eq!(entry.server, "https://198.51.100.7");
    assert!(entry.exec.command.ends_with("gke-gcloud-auth-plugin"));
}

#[tokio::test]
async fn a_missing_plugin_blocks_the_entry_with_the_install_command() {
    let dir = TempDir::new().expect("temp dir");
    let gke = adapter(&dir, false);
    let scope = CloudScope::Gcp {
        project: "p1".into(),
    };
    let clusters = gke.list_clusters(&scope).await.expect("list");
    let error = gke
        .kubeconfig_entry(&clusters[0])
        .await
        .expect_err("no plugin");
    assert!(
        error
            .message()
            .contains("gcloud components install gke-gcloud-auth-plugin")
    );
}

/// A `gcloud` that takes a second per project, leaving a marker only when it ran to the end.
fn slow_adapter(dir: &TempDir) -> GcloudDiscovery {
    let body = format!(
        r#"
case "$*" in
  "container clusters list --project p1 --format=json"|"container clusters list --project p2 --format=json")
    sleep 1; touch "{}/done-$5"; echo '[]' ;;
  *) exit 2 ;;
esac
"#,
        dir.path().display()
    );
    let paths = BTreeMap::from([(Cli::Gcloud, script(dir.path(), "gcloud", &body))]);
    let config = CliRunnerConfig {
        paths,
        login_shell: LoginShell::Disabled,
        ..CliRunnerConfig::default()
    };
    GcloudDiscovery::new(Arc::new(CliRunner::new(config)))
}

fn gcp(project: &str) -> CloudScope {
    CloudScope::Gcp {
        project: project.into(),
    }
}

#[tokio::test]
async fn two_projects_list_in_parallel() {
    let dir = TempDir::new().expect("temp dir");
    let gke = slow_adapter(&dir);
    let (p1, p2) = (gcp("p1"), gcp("p2"));
    let started = std::time::Instant::now();
    let (first, second) = tokio::join!(gke.list_clusters(&p1), gke.list_clusters(&p2));
    first.expect("p1");
    second.expect("p2");
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1900),
        "two 1 s listings took {:?}: they ran one after the other",
        started.elapsed()
    );
    assert!(dir.path().join("done-p1").exists() && dir.path().join("done-p2").exists());
}

#[tokio::test]
async fn dropping_the_listing_cancels_both_processes() {
    let dir = TempDir::new().expect("temp dir");
    let gke = slow_adapter(&dir);
    let (p1, p2) = (gcp("p1"), gcp("p2"));
    let both = async { tokio::join!(gke.list_clusters(&p1), gke.list_clusters(&p2)) };
    let cancelled = tokio::time::timeout(std::time::Duration::from_millis(300), both).await;
    assert!(
        cancelled.is_err(),
        "the listings finished before the cancel"
    );
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert!(
        !dir.path().join("done-p1").exists() && !dir.path().join("done-p2").exists(),
        "a cancelled listing left its process running"
    );
}
