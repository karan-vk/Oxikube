//! `AwsDiscovery` on recorded `aws` output (`tests/fixtures/aws-2.15/`): no CLI or account.

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{
    CloudDiscoveryPort, CloudScope, CloudToolStatus, ClusterStatus, ExecInteractiveMode,
};
use oxikube_testkit::contracts::cloud_discovery;

use super::{AwsDiscovery, AwsSettings};
use crate::process::{Cli, ProcessRunner};
use crate::testing::{RecordedRunner, Reply};

fn fixture(name: &str) -> Reply {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/aws-2.15")
        .join(name);
    Reply::stdout_file(path)
}

fn scope(profile: &str, region: &str) -> CloudScope {
    CloudScope::Aws {
        profile: profile.into(),
        region: region.into(),
    }
}

/// A runner with profiles `default` and `prod`, region `eu-west-1` on `prod` only.
fn recorded() -> RecordedRunner {
    let mut runner = RecordedRunner::new()
        .installed(Cli::Aws)
        .on(Cli::Aws, &["--version"], fixture("version.txt"))
        .on(
            Cli::Aws,
            &["configure", "list-profiles"],
            Reply::Stdout("default\nprod\n".into()),
        )
        .on(
            Cli::Aws,
            &["configure", "get", "region"],
            Reply::fail(1, ""),
        )
        .on(
            Cli::Aws,
            &["configure", "get", "region", "--profile", "prod"],
            Reply::Stdout("eu-west-1\n".into()),
        );
    for args in [
        &["sts", "get-caller-identity"][..],
        &["sts", "get-caller-identity", "--profile", "prod"][..],
    ] {
        runner = runner.on(Cli::Aws, args, fixture("sts-get-caller-identity.json"));
    }
    runner = runner.on(
        Cli::Aws,
        &[
            "eks",
            "list-clusters",
            "--region",
            "eu-west-1",
            "--profile",
            "prod",
        ],
        fixture("eks-list-clusters.json"),
    );
    for (name, file) in [
        ("prod", "eks-describe-cluster-prod.json"),
        ("staging", "eks-describe-cluster-staging.json"),
    ] {
        runner = runner.on(
            Cli::Aws,
            &[
                "eks",
                "describe-cluster",
                "--name",
                name,
                "--region",
                "eu-west-1",
                "--profile",
                "prod",
            ],
            fixture(file),
        );
    }
    runner
}

fn discovery(runner: RecordedRunner, settings: AwsSettings) -> (AwsDiscovery, Arc<RecordedRunner>) {
    let runner = Arc::new(runner);
    let shared: Arc<dyn ProcessRunner> = runner.clone();
    (AwsDiscovery::new(shared, settings), runner)
}

#[tokio::test]
async fn detects_v2_with_logged_in_profiles() {
    let (aws, _) = discovery(recorded(), AwsSettings::default());
    let detection = aws.detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::Ready);
    assert_eq!(detection.version.as_deref(), Some("2.15.30"));
    assert!(detection.binary.is_some() && detection.hints.is_empty());
}

#[tokio::test]
async fn rejects_v1_with_an_actionable_message() {
    let runner = RecordedRunner::new().installed(Cli::Aws).on(
        Cli::Aws,
        &["--version"],
        Reply::Stdout("aws-cli/1.27.0 Python/3.9 Linux/5 botocore/1.29".into()),
    );
    let (aws, _) = discovery(runner, AwsSettings::default());
    let detection = aws.detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::NotInstalled);
    assert_eq!(detection.version.as_deref(), Some("1.27.0"));
    assert!(detection.hints[0].summary.contains("v2 is required"));
}

#[tokio::test]
async fn a_missing_cli_is_not_installed() {
    let (aws, _) = discovery(RecordedRunner::new(), AwsSettings::default());
    let detection = aws.detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::NotInstalled);
}

#[tokio::test]
async fn expired_sso_needs_login_with_the_remediation() {
    let runner = RecordedRunner::new()
        .installed(Cli::Aws)
        .on(Cli::Aws, &["--version"], fixture("version.txt"))
        .on(
            Cli::Aws,
            &["configure", "list-profiles"],
            Reply::Stdout("sso\n".into()),
        )
        .on(
            Cli::Aws,
            &["sts", "get-caller-identity", "--profile", "sso"],
            Reply::fail(255, "The SSO session associated with this profile has expired or is otherwise invalid. To refresh this SSO session run aws sso login"),
        );
    let (aws, _) = discovery(runner, AwsSettings::default());
    let detection = aws.detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::NotAuthenticated);
    assert_eq!(
        detection.hints[0].command.as_deref(),
        Some("aws sso login --profile sso")
    );
}

#[tokio::test]
async fn scopes_combine_settings_regions_and_profile_defaults() {
    let settings = AwsSettings {
        profiles: Vec::new(),
        regions: vec!["us-east-1".into()],
    };
    let (aws, _) = discovery(recorded(), settings);
    let scopes = aws.scopes().await.unwrap();
    assert_eq!(
        scopes,
        vec![
            scope("default", "us-east-1"),
            scope("prod", "eu-west-1"),
            scope("prod", "us-east-1"),
        ]
    );
}

#[tokio::test]
async fn settings_narrow_the_profiles() {
    let settings = AwsSettings {
        profiles: vec!["prod".into()],
        regions: Vec::new(),
    };
    let (aws, runner) = discovery(recorded(), settings);
    assert_eq!(
        aws.scopes().await.unwrap(),
        vec![scope("prod", "eu-west-1")]
    );
    assert!(
        runner
            .calls()
            .iter()
            .all(|call| call.args != ["configure", "list-profiles"])
    );
}

#[tokio::test]
async fn new_settings_apply_to_the_next_scan() {
    let (aws, _) = discovery(recorded(), AwsSettings::default());
    assert_eq!(aws.scopes().await.unwrap().len(), 1);
    aws.set_settings(AwsSettings {
        profiles: vec!["prod".into()],
        regions: vec!["ap-northeast-1".into()],
    });
    assert_eq!(
        aws.scopes().await.unwrap(),
        vec![scope("prod", "ap-northeast-1"), scope("prod", "eu-west-1")]
    );
}

#[tokio::test]
async fn no_regions_is_a_clear_error_not_an_empty_success() {
    let settings = AwsSettings {
        profiles: vec!["default".into()],
        regions: Vec::new(),
    };
    let (aws, _) = discovery(recorded(), settings);
    let error: OxiError = aws.scopes().await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(error.message().contains("no AWS regions"));
}

#[tokio::test]
async fn lists_and_describes_clusters() {
    let (aws, runner) = discovery(recorded(), AwsSettings::default());
    let clusters = aws
        .list_clusters(&scope("prod", "eu-west-1"))
        .await
        .unwrap();
    assert_eq!(clusters.len(), 2);
    let prod = &clusters[0];
    assert_eq!(prod.name, "prod");
    assert_eq!(prod.location, "eu-west-1");
    assert_eq!(prod.status, ClusterStatus::Running);
    assert_eq!(prod.kubernetes_version.as_deref(), Some("1.29"));
    assert_eq!(
        prod.native_id.as_deref(),
        Some("arn:aws:eks:eu-west-1:000000000000:cluster/prod")
    );
    assert!(prod.endpoint.as_deref().unwrap().starts_with("https://"));
    assert!(prod.ca_data.is_some());
    let staging = &clusters[1];
    assert_eq!(staging.status, ClusterStatus::Creating);
    assert!(staging.endpoint.is_none() && staging.ca_data.is_none());
    let describes = runner
        .calls()
        .iter()
        .filter(|call| call.args.get(1).is_some_and(|a| a == "describe-cluster"))
        .count();
    assert_eq!(describes, 2);
}

#[tokio::test]
async fn a_cluster_deleted_mid_listing_is_skipped() {
    let runner = recorded().on(
        Cli::Aws,
        &[
            "eks",
            "describe-cluster",
            "--name",
            "staging",
            "--region",
            "eu-west-1",
            "--profile",
            "prod",
        ],
        Reply::fail(
            254,
            "An error occurred (ResourceNotFoundException) when calling DescribeCluster",
        ),
    );
    let (aws, _) = discovery(runner, AwsSettings::default());
    let clusters = aws
        .list_clusters(&scope("prod", "eu-west-1"))
        .await
        .unwrap();
    assert_eq!(clusters.len(), 1);
}

#[tokio::test]
async fn list_errors_are_classified() {
    let runner = recorded().on(
        Cli::Aws,
        &[
            "eks",
            "list-clusters",
            "--region",
            "eu-west-1",
            "--profile",
            "prod",
        ],
        Reply::fail(
            255,
            "An error occurred (AccessDeniedException): not authorized",
        ),
    );
    let (aws, _) = discovery(runner, AwsSettings::default());
    let error = aws
        .list_clusters(&scope("prod", "eu-west-1"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
}

#[tokio::test]
async fn entries_are_exec_auth_with_the_profile_and_region() {
    let (aws, _) = discovery(recorded(), AwsSettings::default());
    let clusters = aws
        .list_clusters(&scope("prod", "eu-west-1"))
        .await
        .unwrap();
    let entry = aws.kubeconfig_entry(&clusters[0]).await.unwrap();
    assert_eq!(entry.context.as_str(), "eks:prod:eu-west-1:prod");
    assert_eq!(entry.server, clusters[0].endpoint.clone().unwrap());
    assert_eq!(entry.ca_data, clusters[0].ca_data);
    assert_eq!(entry.exec.command, "aws");
    assert_eq!(
        entry.exec.args,
        [
            "eks",
            "get-token",
            "--cluster-name",
            "prod",
            "--region",
            "eu-west-1",
            "--profile",
            "prod"
        ]
    );
    assert_eq!(
        entry.exec.interactive_mode,
        ExecInteractiveMode::IfAvailable
    );
    assert_eq!(
        entry.exec.api_version,
        "client.authentication.k8s.io/v1beta1"
    );
}

#[tokio::test]
async fn the_default_profile_is_left_off_the_exec_args() {
    let (aws, _) = discovery(recorded(), AwsSettings::default());
    let mut cluster = aws
        .list_clusters(&scope("prod", "eu-west-1"))
        .await
        .unwrap()
        .remove(0);
    cluster.scope = scope("default", "eu-west-1");
    let entry = aws.kubeconfig_entry(&cluster).await.unwrap();
    assert!(!entry.exec.args.iter().any(|a| a == "--profile"));
}

#[tokio::test]
async fn a_cluster_without_an_endpoint_has_no_entry() {
    let (aws, _) = discovery(recorded(), AwsSettings::default());
    let clusters = aws
        .list_clusters(&scope("prod", "eu-west-1"))
        .await
        .unwrap();
    let error = aws.kubeconfig_entry(&clusters[1]).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn satisfies_the_discovery_contract() {
    let settings = AwsSettings {
        profiles: vec!["prod".into()],
        regions: Vec::new(),
    };
    // A cluster still being created has no endpoint to build an entry from, so the contract
    // (an entry for every listed cluster) is run on a listing of the active one.
    let runner = recorded().on(
        Cli::Aws,
        &[
            "eks",
            "list-clusters",
            "--region",
            "eu-west-1",
            "--profile",
            "prod",
        ],
        Reply::Stdout(r#"{"clusters":["prod"]}"#.into()),
    );
    let (aws, _) = discovery(runner, settings);
    let report = cloud_discovery(&aws).await;
    assert!(report.scopes > 0 && report.clusters > 0, "{report:?}");
    let (missing, _) = discovery(RecordedRunner::new(), AwsSettings::default());
    cloud_discovery(&missing).await;
}

#[tokio::test]
async fn repeated_profiles_yield_each_scope_once() {
    let settings = AwsSettings {
        profiles: vec!["prod".into(), "default".into(), "prod".into()],
        regions: vec!["eu-west-1".into()],
    };
    let (aws, _) = discovery(recorded(), settings);
    let scopes = aws.scopes().await.unwrap();
    assert_eq!(
        scopes,
        vec![scope("prod", "eu-west-1"), scope("default", "eu-west-1")]
    );
}

#[tokio::test]
async fn the_implicit_default_profile_is_never_named_on_the_command_line() {
    let runner = RecordedRunner::new()
        .installed(Cli::Aws)
        .on(Cli::Aws, &["--version"], fixture("version.txt"))
        .on(
            Cli::Aws,
            &["configure", "list-profiles"],
            Reply::Stdout(String::new()),
        )
        .on(
            Cli::Aws,
            &["sts", "get-caller-identity"],
            fixture("sts-get-caller-identity.json"),
        );
    let (aws, runner) = discovery(runner, AwsSettings::default());
    let detection = aws.detect().await.unwrap();
    assert_eq!(detection.status, CloudToolStatus::Ready);
    assert!(
        runner
            .calls()
            .iter()
            .all(|call| !call.args.iter().any(|a| a == "--profile"))
    );
}
