//! GKE adapter tests on recorded `gcloud` output (`tests/fixtures/gcloud-472.0.0/`).

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_ports::cloud::{
    CloudDiscoveryPort, CloudProvider, CloudScope, CloudToolStatus, ClusterStatus,
    ExecInteractiveMode,
};

use super::GcloudDiscovery;
use crate::process::Cli;
use crate::testing::{RecordedRunner, Reply};

fn fixture(name: &str) -> Reply {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gcloud-472.0.0");
    Reply::stdout_file(dir.join(name))
}

fn stderr_fixture(name: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gcloud-472.0.0");
    std::fs::read_to_string(dir.join(name)).expect("fixture exists")
}

fn project(id: &str) -> CloudScope {
    CloudScope::Gcp { project: id.into() }
}

const LIST: [&str; 5] = [
    "container",
    "clusters",
    "list",
    "--project",
    "acme-platform-1",
];

fn base() -> RecordedRunner {
    RecordedRunner::new()
        .installed(Cli::Gcloud)
        .installed(Cli::GkeGcloudAuthPlugin)
        .on(Cli::Gcloud, &["version"], fixture("version.json"))
        .on(Cli::Gcloud, &["auth", "list"], fixture("auth-list.json"))
        .on(
            Cli::Gcloud,
            &["projects", "list"],
            fixture("projects-list.json"),
        )
        .on(Cli::Gcloud, &LIST, fixture("clusters-list.json"))
}

fn adapter(runner: RecordedRunner) -> GcloudDiscovery {
    GcloudDiscovery::new(Arc::new(runner))
}

#[tokio::test]
async fn missing_gcloud_is_not_installed_not_an_error() {
    let detection = adapter(RecordedRunner::new()).detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::NotInstalled);
    assert!(detection.binary.is_none());
}

#[tokio::test]
async fn detect_reports_version_login_and_plugin() {
    let detection = adapter(base()).detect().await.unwrap();
    assert_eq!(detection.provider, CloudProvider::Gcp);
    assert_eq!(detection.status, CloudToolStatus::Ready);
    assert_eq!(detection.version.as_deref(), Some("472.0.0"));
    assert!(detection.binary.is_some());
    assert!(
        detection
            .hints
            .iter()
            .all(|h| !h.summary.contains("gke-gcloud-auth-plugin")),
        "{:?}",
        detection.hints
    );
}

#[tokio::test]
async fn detect_flags_a_missing_plugin_but_stays_ready() {
    let runner = RecordedRunner::new()
        .installed(Cli::Gcloud)
        .on(Cli::Gcloud, &["version"], fixture("version.json"))
        .on(Cli::Gcloud, &["auth", "list"], fixture("auth-list.json"));
    let detection = adapter(runner).detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::Ready);
    let hint = detection
        .hints
        .iter()
        .find(|h| h.summary.contains("gke-gcloud-auth-plugin"))
        .expect("a plugin hint");
    assert_eq!(
        hint.command.as_deref(),
        Some("gcloud components install gke-gcloud-auth-plugin")
    );
}

#[tokio::test]
async fn no_active_account_is_not_authenticated() {
    let runner = RecordedRunner::new()
        .installed(Cli::Gcloud)
        .installed(Cli::GkeGcloudAuthPlugin)
        .on(Cli::Gcloud, &["version"], fixture("version.json"))
        .on(Cli::Gcloud, &["auth", "list"], Reply::Stdout("[]".into()));
    let detection = adapter(runner).detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::NotAuthenticated);
    assert!(
        detection
            .hints
            .iter()
            .any(|h| h.command.as_deref() == Some("gcloud auth login"))
    );
}

#[tokio::test]
async fn scopes_are_active_projects_with_the_config_project_first() {
    let runner = base().on(
        Cli::Gcloud,
        &["config", "get-value", "project"],
        Reply::Stdout("acme-platform-1\n".into()),
    );
    let scopes = adapter(runner).scopes().await.unwrap();
    assert_eq!(scopes, [project("acme-platform-1"), project("acme-data-3")]);

    let runner = base().on(
        Cli::Gcloud,
        &["config", "get-value", "project"],
        Reply::Stdout("acme-data-3".into()),
    );
    let scopes = adapter(runner).scopes().await.unwrap();
    assert_eq!(scopes, [project("acme-data-3"), project("acme-platform-1")]);

    // Unset (or a failing `config`): plain sorted order.
    let scopes = adapter(base()).scopes().await.unwrap();
    assert_eq!(scopes, [project("acme-data-3"), project("acme-platform-1")]);
}

#[tokio::test]
async fn lists_clusters_across_locations_and_tolerates_unknown_fields() {
    let scope = project("acme-platform-1");
    let clusters = adapter(base()).list_clusters(&scope).await.unwrap();
    assert_eq!(clusters.len(), 3);
    let regional = &clusters[0];
    assert_eq!(regional.location, "europe-west1");
    assert_eq!(regional.endpoint.as_deref(), Some("https://203.0.113.10"));
    assert_eq!(
        regional.kubernetes_version.as_deref(),
        Some("1.29.4-gke.1043002")
    );
    assert_eq!(regional.status, ClusterStatus::Running);
    assert!(regional.ca_data.is_some() && regional.native_id.is_some());
    assert_eq!(regional.scope, scope);
    assert_eq!(clusters[1].status, ClusterStatus::Updating);
    assert_eq!(clusters[2].status, ClusterStatus::Creating);
    assert!(clusters[2].endpoint.is_none());
    // The same name in two locations stays apart.
    assert_ne!(clusters[0].key(), clusters[1].key());
}

#[tokio::test]
async fn a_disabled_container_api_says_how_to_enable_it() {
    let runner = base().on(
        Cli::Gcloud,
        &LIST,
        Reply::fail(1, stderr_fixture("api-disabled.stderr")),
    );
    let error = adapter(runner)
        .list_clusters(&project("acme-platform-1"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        error
            .message()
            .contains("gcloud services enable container.googleapis.com --project acme-platform-1"),
        "{}",
        error.message()
    );
}

#[tokio::test]
async fn listing_failures_are_classified() {
    let denied = base().on(
        Cli::Gcloud,
        &LIST,
        Reply::fail(1, stderr_fixture("permission-denied.stderr")),
    );
    let error = adapter(denied)
        .list_clusters(&project("acme-platform-1"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);

    let logged_out = base().on(
        Cli::Gcloud,
        &LIST,
        Reply::fail(1, "ERROR: (gcloud.container.clusters.list) You do not currently have an active account selected."),
    );
    let error = adapter(logged_out)
        .list_clusters(&project("acme-platform-1"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Auth);
    assert!(error.message().contains("gcloud auth login"));

    let slow = base().on(Cli::Gcloud, &LIST, Reply::Timeout);
    let error = adapter(slow)
        .list_clusters(&project("acme-platform-1"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Timeout);
}

#[tokio::test]
async fn foreign_scopes_and_option_like_projects_are_rejected() {
    let aws = CloudScope::Aws {
        profile: "p".into(),
        region: "r".into(),
    };
    let error = adapter(base()).list_clusters(&aws).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    let error = adapter(base())
        .list_clusters(&project("--impersonate-service-account=x"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn entry_uses_the_exec_plugin_and_a_deterministic_context() {
    let scope = project("acme-platform-1");
    let adapter = adapter(base());
    let clusters = adapter.list_clusters(&scope).await.unwrap();
    let entry = adapter.kubeconfig_entry(&clusters[1]).await.unwrap();
    assert_eq!(
        entry.context.as_str(),
        "gke:acme-platform-1:us-central1-b:prod"
    );
    assert_eq!(entry.server, "https://203.0.113.20");
    assert!(entry.ca_data.is_some());
    assert!(entry.exec.command.ends_with("gke-gcloud-auth-plugin"));
    assert!(entry.exec.args.is_empty() && entry.exec.env.is_empty());
    assert!(entry.exec.provide_cluster_info);
    assert_eq!(
        entry.exec.interactive_mode,
        ExecInteractiveMode::IfAvailable
    );
    assert_eq!(
        entry.exec.api_version,
        "client.authentication.k8s.io/v1beta1"
    );
    assert!(
        entry
            .exec
            .install_hint
            .as_deref()
            .is_some_and(|h| h.contains("gcloud components install gke-gcloud-auth-plugin"))
    );
}

#[tokio::test]
async fn entry_describes_the_cluster_when_the_listing_lacked_endpoint_and_ca() {
    let describe = [
        "container",
        "clusters",
        "describe",
        "fresh",
        "--location",
        "us-east1",
        "--project",
        "acme-platform-1",
    ];
    let runner = base().on(
        Cli::Gcloud,
        &describe,
        fixture("clusters-describe-fresh.json"),
    );
    let adapter = adapter(runner);
    let clusters = adapter
        .list_clusters(&project("acme-platform-1"))
        .await
        .unwrap();
    let entry = adapter.kubeconfig_entry(&clusters[2]).await.unwrap();
    assert_eq!(entry.server, "https://203.0.113.30");
    assert_eq!(entry.ca_data.as_deref(), Some("RkVSU0g="));
    assert_eq!(entry.context.as_str(), "gke:acme-platform-1:us-east1:fresh");
}

#[tokio::test]
async fn entry_without_the_plugin_is_an_actionable_unsupported_error() {
    let runner = RecordedRunner::new().installed(Cli::Gcloud).on(
        Cli::Gcloud,
        &LIST,
        fixture("clusters-list.json"),
    );
    let adapter = adapter(runner);
    let clusters = adapter
        .list_clusters(&project("acme-platform-1"))
        .await
        .unwrap();
    let error = adapter.kubeconfig_entry(&clusters[0]).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    let message = error.message();
    assert!(
        message.contains("gcloud components install gke-gcloud-auth-plugin"),
        "{message}"
    );
    assert!(
        message.contains("cloud.gke_gcloud_auth_plugin_path"),
        "{message}"
    );
}

#[tokio::test]
async fn no_command_prints_a_credential() {
    let runner = Arc::new(base().on(
        Cli::Gcloud,
        &["config", "get-value", "project"],
        Reply::Stdout("acme-platform-1".into()),
    ));
    let adapter = GcloudDiscovery::new(runner.clone());
    adapter.detect().await.unwrap();
    adapter.scopes().await.unwrap();
    let clusters = adapter
        .list_clusters(&project("acme-platform-1"))
        .await
        .unwrap();
    adapter.kubeconfig_entry(&clusters[0]).await.unwrap();
    for call in runner.calls() {
        let line = call.args.join(" ");
        assert!(
            !line.contains("token") && !line.contains("print-access"),
            "{line}"
        );
    }
}

#[tokio::test]
async fn configured_projects_are_the_scopes_and_skip_the_projects_call() {
    let runner = Arc::new(base());
    let gke = GcloudDiscovery::new(runner.clone());
    gke.set_projects(vec!["only-this".into()]);
    assert_eq!(gke.scopes().await.unwrap(), vec![project("only-this")]);
    assert!(
        runner
            .calls()
            .iter()
            .all(|call| call.args != ["projects", "list"]),
        "naming projects must not run `gcloud projects list`"
    );
    gke.set_projects(Vec::new());
    assert!(gke.scopes().await.unwrap().len() > 1);
}
