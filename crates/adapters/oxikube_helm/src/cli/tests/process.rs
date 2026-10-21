//! The process behaviour against a fake `helm`: detection and the version gate, cluster flags,
//! values on stdin, streaming order, timeouts, kill on drop and exit-code mapping.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::helm::{
    ChartRef, HelmVersion, InstallRequest, OutputLine, ReleaseRef, RollbackRequest, SearchQuery,
    UpgradeRequest, Values,
};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::HelmPort;

use super::fake::{FakeHelm, cluster, gone, pid};
use super::fixture;
use crate::cli::{HelmCli, HelmCliConfig, detect};

fn release() -> ReleaseRef {
    ReleaseRef::new(cluster(), "web", "shop")
}

#[tokio::test]
async fn detection_reports_the_version_and_caches_it() {
    let fake = FakeHelm::new("v3.14.2+gabc", "exit 0");
    let cli = fake.cli();
    assert!(!cli.capabilities().cli_available, "nothing detected yet");
    let caps = cli.detect().await;
    assert!(caps.cli_available && caps.can_install && caps.can_search_repos);
    assert!(
        !caps.native_read,
        "the CLI adapter does not read release Secrets itself"
    );
    assert_eq!(caps.cli_version, Some(HelmVersion::new(3, 14, 2)));
    assert_eq!(cli.capabilities(), caps);
}

#[tokio::test]
async fn the_first_operation_detects() {
    let fake = FakeHelm::new("v4.3.1+gc0d08d0", "echo '[]'");
    let cli = fake.cli();
    assert!(cli.repos().await.unwrap().is_empty());
    assert_eq!(
        cli.capabilities().cli_version,
        Some(HelmVersion::new(4, 3, 1))
    );
}

#[tokio::test]
async fn a_too_old_binary_is_unsupported() {
    let fake = FakeHelm::new("v3.9.4+gdbc6d8e", "exit 0");
    let cli = fake.cli();
    let caps = cli.detect().await;
    assert_eq!(caps.cli_version, Some(HelmVersion::new(3, 9, 4)));
    assert!(!caps.cli_available && !caps.can_mutate());
    let error = cli.repos().await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("3.9.4") && error.message().contains("3.10.0"));
    assert!(fake.argv().is_none(), "nothing but `version` ran");
}

#[tokio::test]
async fn a_missing_binary_hides_the_cli_and_settings_bring_it_back() {
    let fake = FakeHelm::new("v3.10.3+g835b733", "echo '[]'");
    let cli = HelmCli::new(
        HelmCliConfig {
            path: Some(fake.dir().join("not-there")),
            ..HelmCliConfig::default()
        },
        std::sync::Arc::new(|_: &oxikube_domain::ids::ClusterId| None),
    );
    assert_eq!(
        cli.detect().await,
        oxikube_domain::helm::HelmCapabilities::none()
    );
    let error = cli.search(&SearchQuery::default()).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("not found"), "{}", error.message());
    // The settings change: re-detected.
    let caps = cli
        .reconfigure(HelmCliConfig {
            path: Some(fake.path()),
            ..HelmCliConfig::default()
        })
        .await;
    assert!(caps.cli_available);
    assert!(
        cli.search(&SearchQuery::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn path_lookup_finds_helm_on_path() {
    let fake = FakeHelm::new("v3.10.3", "exit 0");
    let path_var =
        std::env::join_paths([std::path::Path::new("/nonexistent"), fake.dir()]).unwrap();
    assert_eq!(detect::locate(None, Some(&path_var)), Some(fake.path()));
    assert_eq!(
        detect::locate(Some(&fake.dir().join("nope")), Some(&path_var)),
        None
    );
}

#[tokio::test]
async fn reads_pass_the_cluster_and_parse_json() {
    let list = fixture("helm-4.3", "list.out");
    let fake = FakeHelm::new("v4.3.1", &format!("cat <<'EOF'\n{list}\nEOF"));
    let cli = fake.cli();
    let releases = cli
        .list_releases(&cluster(), &NamespaceSelection::single("web"))
        .await
        .unwrap();
    assert_eq!(releases.len(), 2);
    let argv = fake.argv().unwrap();
    assert_eq!(argv[0], "list");
    assert!(argv.contains(&"--kube-context=kind-oxikube".to_owned()));
    assert!(argv.contains(&"--namespace=web".to_owned()));
}

#[tokio::test]
async fn an_unknown_cluster_is_not_found() {
    let fake = FakeHelm::new("v3.14.2", "echo '[]'");
    let cli = HelmCli::new(
        HelmCliConfig {
            path: Some(fake.path()),
            ..HelmCliConfig::default()
        },
        std::sync::Arc::new(|_: &oxikube_domain::ids::ClusterId| None),
    );
    let error = cli.history(&release()).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(fake.argv().is_none());
}

#[tokio::test]
async fn no_repositories_is_an_empty_list() {
    let fake = FakeHelm::new(
        "v3.10.3",
        "echo 'Error: no repositories to show' >&2; exit 1",
    );
    assert!(fake.cli().repos().await.unwrap().is_empty());
    let fake = FakeHelm::new(
        "v3.10.3",
        "echo 'Error: no repositories configured' >&2; exit 1",
    );
    assert!(
        fake.cli()
            .search(&SearchQuery::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn exit_codes_map_by_what_helm_said() {
    let fake = FakeHelm::new("v3.14.2", "echo 'Error: release: not found' >&2; exit 1");
    let error = fake.cli().history(&release()).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message().contains("shop"));

    let fake = FakeHelm::new("v3.14.2", "echo 'Error: boom' >&2; exit 3");
    let error = fake.cli().manifest(&release(), Some(1)).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(error.message().contains("exit code Some(3)") && error.message().contains("boom"));

    let fake = FakeHelm::new("v3.14.2", "exit 2");
    let error = fake.cli().manifest(&release(), None).await.unwrap_err();
    assert!(
        error.message().contains("helm exited with"),
        "{}",
        error.message()
    );
}

fn secret_values() -> Values {
    Values::from_yaml("auth:\n  password: s3cr3t-hunter2\nimage:\n  tag: \"1.25\"\n").unwrap()
}

#[tokio::test]
async fn values_go_to_stdin_and_never_come_back_out() {
    // The fake echoes its stdin to stderr and fails, the worst case for a leak.
    let fake = FakeHelm::new(
        "v3.14.2",
        "echo \"Error: bad values: $(cat \"$(dirname \"$0\")/stdin\")\" >&2; exit 1",
    );
    let mut request = InstallRequest::new(cluster(), "shop", "web", ChartRef::new("./demo", ""));
    request.values = secret_values();
    let error = fake.cli().template(&request).await.unwrap_err();
    let argv = fake.argv().unwrap().join(" ");
    assert!(argv.contains("--values=-"));
    assert!(!argv.contains("s3cr3t-hunter2"), "argv: {argv}");
    assert!(
        fake.stdin().unwrap().contains("s3cr3t-hunter2"),
        "values reach helm on stdin"
    );
    assert!(
        !error.message().contains("s3cr3t-hunter2"),
        "{}",
        error.message()
    );
    assert!(!format!("{error:?}").contains("s3cr3t-hunter2"));
}

#[tokio::test]
async fn install_streams_lines_in_order_and_redacts_them() {
    let body = "echo 'NAME: shop'; sleep 0.1; echo 'waiting s3cr3t-hunter2' >&2; sleep 0.1; \
                echo 'STATUS: deployed'";
    let fake = FakeHelm::new("v3.14.2", body);
    let mut request = InstallRequest::new(cluster(), "shop", "web", ChartRef::new("./demo", ""));
    request.values = secret_values();
    let stream = fake.cli().install(&request).await.unwrap();
    assert!(
        fake.argv().is_none(),
        "nothing runs until the stream is polled"
    );
    let lines: Vec<OutputLine> = stream.map(Result::unwrap).collect().await;
    assert_eq!(
        lines,
        [
            OutputLine::stdout("NAME: shop"),
            OutputLine::stderr("waiting [redacted]"),
            OutputLine::stdout("STATUS: deployed"),
        ]
    );
    assert!(fake.stdin().unwrap().contains("password: s3cr3t-hunter2"));
}

#[tokio::test]
async fn a_failed_upgrade_ends_its_stream_with_the_error() {
    let inuse = fixture("helm-4.3", "install_inuse.err");
    let fake = FakeHelm::new(
        "v4.3.1",
        &format!("echo 'progress'; cat >&2 <<'EOF'\n{inuse}\nEOF\nexit 1"),
    );
    let request = UpgradeRequest::new(release(), ChartRef::new("./demo", ""));
    let items: Vec<_> = fake.cli().upgrade(&request).await.unwrap().collect().await;
    assert_eq!(items[0].as_ref().unwrap(), &OutputLine::stdout("progress"));
    assert!(
        items[..items.len() - 1].iter().all(Result::is_ok),
        "{items:?}"
    );
    let error = items.last().unwrap().as_ref().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Conflict);
}

#[tokio::test]
async fn a_hung_read_times_out_and_is_killed() {
    let fake = FakeHelm::new("v3.14.2", "echo $$ > \"$d/pid\"; exec sleep 30");
    let cli = fake.cli_with(Duration::from_millis(300));
    let error = cli.repos().await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Timeout);
    assert!(
        gone(&pid(&fake).await).await,
        "the child is killed on timeout"
    );
}

#[tokio::test]
async fn a_hung_stream_times_out() {
    let fake = FakeHelm::new("v3.14.2", "echo started; exec sleep 30");
    let mut request = RollbackRequest::new(release(), 1);
    request.timeout = Duration::from_millis(300);
    let error = fake.cli().rollback(&request).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Timeout);

    let mut install = InstallRequest::new(cluster(), "shop", "web", ChartRef::new("./demo", ""));
    install.timeout = Duration::from_millis(300);
    let items: Vec<_> = fake.cli().install(&install).await.unwrap().collect().await;
    assert_eq!(items[0].as_ref().unwrap(), &OutputLine::stdout("started"));
    assert_eq!(items[1].as_ref().unwrap_err().kind(), ErrorKind::Timeout);
}

#[tokio::test]
async fn dropping_the_call_kills_helm() {
    let fake = FakeHelm::new("v3.14.2", "echo $$ > \"$d/pid\"; exec sleep 30");
    let cli = fake.cli();
    let target = release();
    let call = cli.manifest(&target, None);
    // Run the call until helm has started, then drop it.
    let pid = tokio::select! {
        result = call => panic!("finished early: {result:?}"),
        pid = pid(&fake) => pid,
    };
    assert!(gone(&pid).await, "kill_on_drop ended it");
}

#[tokio::test]
async fn dropping_the_stream_kills_helm() {
    let fake = FakeHelm::new(
        "v3.14.2",
        "echo $$ > \"$d/pid\"; echo started; exec sleep 30",
    );
    let request = InstallRequest::new(cluster(), "shop", "web", ChartRef::new("./demo", ""));
    let mut stream = fake.cli().install(&request).await.unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        OutputLine::stdout("started")
    );
    let pid = pid(&fake).await;
    drop(stream);
    assert!(
        gone(&pid).await,
        "dropping the output stream kills the install"
    );
}

#[test]
fn helm_env_overrides_are_removed_and_values_are_not_in_it() {
    use std::ffi::OsStr;
    let invocation = crate::cli::run::Invocation::new(Vec::new(), Duration::from_secs(1));
    let command = crate::cli::run::command(std::path::Path::new("helm"), &invocation);
    let envs: Vec<(&OsStr, Option<&OsStr>)> = command.as_std().get_envs().collect();
    for name in [
        "HELM_NAMESPACE",
        "HELM_KUBECONTEXT",
        "HELM_KUBETOKEN",
        "HELM_KUBEAPISERVER",
    ] {
        assert!(
            envs.contains(&(OsStr::new(name), None)),
            "{name} is removed"
        );
    }
    assert!(envs.contains(&(OsStr::new("NO_COLOR"), Some(OsStr::new("1")))));
    assert_eq!(
        envs.len(),
        11,
        "only the scrubbed variables and NO_COLOR: {envs:?}"
    );
}

#[tokio::test]
async fn repo_add_refuses_credentials_in_the_url() {
    let fake = FakeHelm::new("v3.14.2", "exit 0");
    let cli = fake.cli();
    let secret = oxikube_domain::helm::RepoEntry {
        name: "private".into(),
        url: "https://bob:hunter22@charts.example.com/index".into(),
    };
    let error = cli.repo_add(&secret).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(fake.argv().is_none(), "nothing ran");
    let public = oxikube_domain::helm::RepoEntry {
        name: "podinfo".into(),
        url: "https://stefanprodan.github.io/podinfo".into(),
    };
    cli.repo_add(&public).await.unwrap();
    assert_eq!(fake.argv().unwrap()[..3], ["repo", "add", "--"]);
    cli.repo_remove("podinfo").await.unwrap();
    cli.repo_update(&[]).await.unwrap();
    assert_eq!(fake.argv().unwrap(), ["repo", "update"]);
}

#[tokio::test]
async fn a_line_that_is_not_utf8_is_kept_lossily() {
    let fake = FakeHelm::new("v3.14.2", "printf 'ok\\r\\n\\377bad\\n'");
    let request = InstallRequest::new(cluster(), "shop", "web", ChartRef::new("./demo", ""));
    let lines: Vec<_> = fake.cli().install(&request).await.unwrap().collect().await;
    assert_eq!(lines[0].as_ref().unwrap(), &OutputLine::stdout("ok"));
    assert_eq!(
        lines[1].as_ref().unwrap(),
        &OutputLine::stdout("\u{fffd}bad")
    );
    assert_eq!(lines.len(), 2);
}

#[tokio::test]
async fn several_namespaces_are_listed_together() {
    // Each call answers with a release named after its `--namespace=` flag.
    let body = "ns=$(printf '%s\\n' \"$@\" | sed -n 's/^--namespace=//p'); \
                echo \"[{\\\"name\\\":\\\"r\\\",\\\"namespace\\\":\\\"$ns\\\",\\\"revision\\\":\\\"1\\\",\\\"status\\\":\\\"deployed\\\",\\\"chart\\\":\\\"c-1.0.0\\\"}]\"";
    let fake = FakeHelm::new("v3.14.2", body);
    let selection = NamespaceSelection::from_names(["a", "b"]);
    let releases = fake
        .cli()
        .list_releases(&cluster(), &selection)
        .await
        .unwrap();
    let mut namespaces: Vec<&str> = releases
        .iter()
        .map(|r| r.release.namespace.as_str())
        .collect();
    namespaces.sort_unstable();
    assert_eq!(namespaces, ["a", "b"]);
}
