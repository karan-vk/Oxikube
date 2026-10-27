//! `AzureAksAdapter` against a fake `az` and a fake `kubelogin` (shell scripts serving the
//! recorded JSON under `tests/fixtures/az-2.61/`), through the real `CliRunner` (E18-S04). No
//! Azure account, network or real CLI is involved.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxikube_cloud::az::AzureAksAdapter;
use oxikube_cloud::process::{Cli, CliRunner, CliRunnerConfig, LoginShell};
use oxikube_domain::cloud::ExecInteractiveMode;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{
    CloudDiscoveryPort, CloudProvider, CloudScope, CloudToolStatus, ClusterStatus,
    DiscoveredCluster,
};
use oxikube_testkit::contracts::cloud_discovery;
use tempfile::TempDir;

const DEV: &str = "00000000-0000-0000-0000-000000000001";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/az-2.61")
}

fn executable(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("temp dir is writable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("script just written");
}

/// A temp dir with a fake `az` (and optionally `kubelogin`).
struct Fake {
    dir: TempDir,
    adapter: AzureAksAdapter,
}

impl Fake {
    fn new(logged_in: bool, kubelogin: bool, only_cluster: Option<&str>) -> Self {
        let dir = TempDir::new().expect("temp dir");
        let list: Vec<serde_json::Value> =
            serde_json::from_str(&fs::read_to_string(fixtures().join("aks-list.json")).unwrap())
                .unwrap();
        let list: Vec<_> = list
            .into_iter()
            .filter(|c| only_cluster.is_none_or(|name| c["name"] == name))
            .collect();
        fs::write(
            dir.path().join("aks-list.json"),
            serde_json::to_string(&list).unwrap(),
        )
        .unwrap();
        if !logged_in {
            fs::write(dir.path().join("logged_out"), "").unwrap();
        }
        let fix = fixtures();
        let az = dir.path().join("az");
        executable(
            &az,
            &format!(
                r#"FIX='{fix}'; DIR='{dir}'
name=; file=; prev=
for a in "$@"; do
  case "$prev" in --name) name=$a;; --file) file=$a;; esac
  prev=$a
done
case "$1 $2" in
  "version "*) cat "$FIX/version.json";;
  "account show")
    if [ -f "$DIR/logged_out" ]; then
      echo "ERROR: Please run 'az login' to setup account." >&2; exit 1
    fi
    cat "$FIX/account-show.json";;
  "account list") cat "$FIX/account-list.json";;
  "aks list") cat "$DIR/aks-list.json";;
  "aks show")
    case "$name" in
      aad) cat "$FIX/aks-show-aad.json";;
      local) cat "$FIX/aks-show-local.json";;
      idle) cat "$FIX/aks-show-stopped.json";;
      *) echo "ERROR: (ResourceNotFound) not found" >&2; exit 1;;
    esac;;
  "aks get-credentials")
    echo "$file" > "$DIR/credentials_path"
    cp "$FIX/kubeconfig-aad.yaml" "$file"
    echo "Merged $name as current context in $file";;
  *) echo "unexpected: $*" >&2; exit 2;;
esac"#,
                fix = fix.display(),
                dir = dir.path().display()
            ),
        );
        let mut paths = BTreeMap::from([(Cli::Az, az)]);
        let kubelogin_path = dir.path().join("kubelogin");
        if kubelogin {
            executable(&kubelogin_path, "exit 0");
        }
        paths.insert(Cli::Kubelogin, kubelogin_path);
        let runner = CliRunner::new(CliRunnerConfig {
            paths,
            login_shell: LoginShell::Disabled,
            ..CliRunnerConfig::default()
        });
        Self {
            dir,
            adapter: AzureAksAdapter::new(Arc::new(runner)),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    async fn dev_clusters(&self) -> OxiResult<Vec<DiscoveredCluster>> {
        self.adapter.list_clusters(&dev_scope()).await
    }

    async fn cluster(&self, name: &str) -> DiscoveredCluster {
        self.dev_clusters()
            .await
            .expect("listing works")
            .into_iter()
            .find(|c| c.name == name)
            .expect("fixture cluster")
    }
}

fn dev_scope() -> CloudScope {
    CloudScope::Azure {
        subscription_id: DEV.into(),
        subscription_name: Some("Dev".into()),
    }
}

#[tokio::test]
async fn detect_reports_version_login_and_kubelogin() {
    let ready = Fake::new(true, true, None).adapter.detect().await.unwrap();
    assert_eq!(ready.status, CloudToolStatus::Ready);
    assert_eq!(ready.version.as_deref(), Some("2.61.0"));
    assert!(
        ready.binary.is_some() && ready.hints.is_empty(),
        "{ready:?}"
    );

    let out = Fake::new(false, false, None)
        .adapter
        .detect()
        .await
        .unwrap();
    assert_eq!(out.status, CloudToolStatus::NotAuthenticated);
    let commands: Vec<_> = out
        .hints
        .iter()
        .filter_map(|h| h.command.as_deref())
        .collect();
    assert_eq!(commands, ["az login", "az aks install-cli"]);
}

#[tokio::test]
async fn a_missing_az_is_not_installed_not_an_error() {
    let runner = CliRunner::new(CliRunnerConfig {
        paths: BTreeMap::from([(Cli::Az, PathBuf::from("/nonexistent/az"))]),
        login_shell: LoginShell::Disabled,
        ..CliRunnerConfig::default()
    });
    let detection = AzureAksAdapter::new(Arc::new(runner))
        .detect()
        .await
        .unwrap();
    assert_eq!(detection.status, CloudToolStatus::NotInstalled);
    assert!(detection.binary.is_none());
}

#[tokio::test]
async fn scopes_are_the_enabled_subscriptions() {
    let scopes = Fake::new(true, true, None).adapter.scopes().await.unwrap();
    let keys: Vec<_> = scopes.iter().map(CloudScope::key).collect();
    assert_eq!(
        keys,
        [
            format!("azure/{DEV}"),
            "azure/00000000-0000-0000-0000-000000000003".to_owned()
        ]
    );
    assert_eq!(scopes[0].to_string(), "Dev");
}

#[tokio::test]
async fn listing_maps_state_endpoint_and_version() {
    let fake = Fake::new(true, true, None);
    let clusters = fake.dev_clusters().await.unwrap();
    let by_name: BTreeMap<_, _> = clusters.iter().map(|c| (c.name.as_str(), c)).collect();
    let aad = by_name["aad"];
    assert_eq!(aad.status, ClusterStatus::Running);
    assert_eq!(aad.location, "westeurope");
    assert_eq!(aad.kubernetes_version.as_deref(), Some("1.29.4"));
    assert_eq!(
        aad.endpoint.as_deref(),
        Some("https://aad-dns-abc123.hcp.westeurope.azmk8s.io:443")
    );
    assert!(
        aad.native_id
            .as_deref()
            .unwrap()
            .ends_with("/managedClusters/aad")
    );
    assert_eq!(by_name["idle"].status, ClusterStatus::Stopped);
    assert!(!by_name["idle"].status.is_connectable());
    assert_eq!(by_name["new"].status, ClusterStatus::Creating);
    assert!(
        by_name["new"]
            .endpoint
            .as_deref()
            .unwrap()
            .contains("privatelink")
    );
    assert_eq!(by_name["new"].kubernetes_version.as_deref(), Some("1.30"));
}

#[tokio::test]
async fn an_aad_cluster_gets_a_kubelogin_exec_entry_and_no_file_is_left() {
    let fake = Fake::new(true, true, None);
    let cluster = fake.cluster("aad").await;
    let entry = fake.adapter.kubeconfig_entry(&cluster).await.unwrap();

    assert_eq!(entry.context.as_str(), format!("aks:{DEV}:rg-aad:aad"));
    assert_eq!(
        entry.server,
        "https://aad-dns-abc123.hcp.westeurope.azmk8s.io:443"
    );
    assert_eq!(
        entry.ca_data.as_deref(),
        Some("LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t")
    );
    assert_eq!(entry.exec.command, fake.path("kubelogin").to_string_lossy());
    assert_eq!(
        entry.exec.args,
        [
            "get-token",
            "--login",
            "azurecli",
            "--server-id",
            "6dae42f8-4368-4678-94ff-3960e28e3630",
            "--tenant-id",
            "11111111-1111-1111-1111-111111111111"
        ]
    );
    assert_eq!(entry.exec.interactive_mode, ExecInteractiveMode::Never);
    let debug = format!("{entry:?}");
    assert!(!debug.contains("MUST-NOT-BE-KEPT"), "{debug}");

    // The credentials file az wrote is gone with its private directory.
    let written = fs::read_to_string(fake.path("credentials_path")).expect("az was asked");
    let written = PathBuf::from(written.trim());
    assert!(!written.exists(), "{} was not deleted", written.display());
    assert!(!written.parent().unwrap().exists());
}

#[tokio::test]
async fn the_credentials_directory_is_private_while_az_writes() {
    let fake = Fake::new(true, true, None);
    // Replace the fake's get-credentials step by one recording the directory mode.
    let az = fake.path("az");
    let script = fs::read_to_string(&az).unwrap().replace(
        "echo \"$file\" > \"$DIR/credentials_path\"",
        "echo \"$file\" > \"$DIR/credentials_path\"; \
         ls -ld \"$(dirname \"$file\")\" | cut -c1-10 > \"$DIR/mode\"",
    );
    fs::write(&az, script).unwrap();
    let cluster = fake.cluster("aad").await;
    fake.adapter.kubeconfig_entry(&cluster).await.unwrap();
    assert_eq!(
        fs::read_to_string(fake.path("mode")).unwrap().trim(),
        "drwx------"
    );
}

#[tokio::test]
async fn local_account_clusters_are_refused_without_fetching_credentials() {
    let fake = Fake::new(true, true, None);
    let cluster = fake.cluster("local").await;
    let error = fake.adapter.kubeconfig_entry(&cluster).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.to_string().contains("Azure AD"), "{error}");
    assert!(!fake.path("credentials_path").exists());
}

#[tokio::test]
async fn stopped_clusters_are_listed_but_not_connectable() {
    let fake = Fake::new(true, true, None);
    let cluster = fake.cluster("idle").await;
    let error = fake.adapter.kubeconfig_entry(&cluster).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.to_string().contains("az aks start"), "{error}");
}

#[tokio::test]
async fn a_missing_kubelogin_is_an_actionable_error() {
    let fake = Fake::new(true, false, None);
    let cluster = fake.cluster("aad").await;
    let error = fake.adapter.kubeconfig_entry(&cluster).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    let text = error.to_string();
    assert!(
        text.contains("kubelogin") && text.contains("azure.github.io/kubelogin"),
        "{text}"
    );
    assert!(text.contains("cloud.kubelogin_path"), "{text}");
    assert!(!fake.path("credentials_path").exists());
}

#[tokio::test]
async fn a_logged_out_cli_is_an_auth_error_with_the_hint() {
    let fake = Fake::new(false, true, None);
    fs::write(
        fake.path("az"),
        "#!/bin/sh\necho \"ERROR: Please run 'az login' to setup account.\" >&2\nexit 1\n",
    )
    .unwrap();
    let error = fake.adapter.scopes().await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Auth);
    assert!(error.to_string().contains("az login"), "{error}");
}

#[tokio::test]
async fn meets_the_cloud_discovery_contract() {
    let fake = Fake::new(true, true, Some("aad"));
    assert_eq!(fake.adapter.id(), CloudProvider::Azure);
    let report = cloud_discovery(&fake.adapter).await;
    assert_eq!((report.scopes, report.clusters), (2, 2));
}
