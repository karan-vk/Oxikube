//! The argv of each command: cluster flags, `--flag=value`, positionals after `--`, no values.

use std::path::PathBuf;
use std::time::Duration;

use oxikube_domain::helm::{
    ChartRef, HelmVersion, InstallRequest, ReleaseRef, RepoAddRequest, RepoPassword,
    RollbackRequest, SearchQuery, ShowKind, UninstallRequest, UpgradeRequest,
};
use oxikube_domain::ids::{ClusterId, ContextName};

use crate::cli::args::{self, Get};
use crate::cli::target::HelmTarget;

fn target() -> HelmTarget {
    HelmTarget::new(
        ContextName::new("kind-oxikube"),
        Some(PathBuf::from("/home/u/.kube/config")),
    )
}

fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::new("kind-oxikube"))
}

fn release() -> ReleaseRef {
    ReleaseRef::new(cluster(), "web", "shop")
}

const CTX: [&str; 2] = [
    "--kube-context=kind-oxikube",
    "--kubeconfig=/home/u/.kube/config",
];

#[test]
fn list_spells_out_every_state_and_scope() {
    let argv = args::list(&target(), Some("web"));
    let mut expected = vec!["list"];
    expected.extend(CTX);
    expected.extend([
        "--output=json",
        "--max=0",
        "--deployed",
        "--failed",
        "--pending",
        "--superseded",
        "--uninstalled",
        "--uninstalling",
        "--namespace=web",
    ]);
    assert_eq!(argv, expected);
    let all = args::list(&HelmTarget::new(ContextName::new("c"), None), None);
    assert!(all.contains(&"--all-namespaces".to_owned()));
    assert!(!all.iter().any(|a| a.starts_with("--kubeconfig")));
    assert!(
        !all.iter().any(|a| a == "--all"),
        "Helm 4 has no `list --all`"
    );
}

#[test]
fn history_and_get() {
    let mut expected = vec!["history"];
    expected.extend(CTX);
    expected.extend(["--namespace=web", "--output=json", "--", "shop"]);
    assert_eq!(args::history(&target(), "web", "shop"), expected);

    let values = args::get(&target(), "web", "shop", Some(3), Get::Values { all: true });
    assert_eq!(&values[..2], ["get", "values"]);
    assert!(values.ends_with(&[
        "--output=json".into(),
        "--all".into(),
        "--revision=3".into(),
        "--".into(),
        "shop".into()
    ]));
    let notes = args::get(&target(), "web", "shop", None, Get::Notes);
    assert_eq!(&notes[..2], ["get", "notes"]);
    assert!(
        !notes
            .iter()
            .any(|a| a.starts_with("--output") || a.starts_with("--revision"))
    );
    assert_eq!(
        args::get(&target(), "web", "shop", None, Get::Manifest)[1],
        "manifest"
    );
}

#[test]
fn a_name_that_looks_like_a_flag_stays_a_positional() {
    let argv = args::history(&target(), "web", "--all-namespaces");
    let dashdash = argv.iter().position(|a| a == "--").unwrap();
    assert_eq!(argv[dashdash + 1], "--all-namespaces");
    let argv = args::get(&target(), "-n x", "r", None, Get::Manifest);
    assert!(argv.contains(&"--namespace=-n x".to_owned()));
}

#[test]
fn repo_search_and_show() {
    assert_eq!(args::repo_list(), ["repo", "list", "--output=json"]);
    assert_eq!(
        args::repo_add(&RepoAddRequest::new(
            "podinfo",
            "https://stefanprodan.github.io/podinfo"
        )),
        [
            "repo",
            "add",
            "--",
            "podinfo",
            "https://stefanprodan.github.io/podinfo"
        ]
    );
    assert_eq!(
        args::repo_remove("podinfo"),
        ["repo", "remove", "--", "podinfo"]
    );
    assert_eq!(args::repo_update(&[]), ["repo", "update"]);
    assert_eq!(
        args::repo_update(&["a".into()]),
        ["repo", "update", "--", "a"]
    );
    let query = SearchQuery {
        keyword: "nginx".into(),
        all_versions: true,
    };
    assert_eq!(
        args::search(&query),
        [
            "search",
            "repo",
            "--output=json",
            "--versions",
            "--",
            "nginx"
        ]
    );
    assert_eq!(
        args::search(&SearchQuery::default()),
        ["search", "repo", "--output=json"]
    );
    let chart = ChartRef::new("nginx", "15.4.0").in_repo("bitnami");
    assert_eq!(
        args::show(&chart, ShowKind::Readme),
        ["show", "readme", "--version=15.4.0", "--", "bitnami/nginx"]
    );
    assert_eq!(
        args::show(&ChartRef::new("./demo", ""), ShowKind::Values),
        ["show", "values", "--", "./demo"]
    );
}

fn install_request() -> InstallRequest {
    let mut request = InstallRequest::new(
        cluster(),
        "shop",
        "web",
        ChartRef::new("nginx", "15.4.0").in_repo("bitnami"),
    );
    request.create_namespace = true;
    request.wait = true;
    request.timeout = Duration::from_secs(120);
    request
}

#[test]
fn install_passes_values_on_stdin_and_picks_the_dry_run_form() {
    let mut request = install_request();
    let argv = args::install(&target(), &request, true, HelmVersion::new(3, 14, 0));
    let mut expected = vec!["install"];
    expected.extend(CTX);
    expected.extend([
        "--namespace=web",
        "--create-namespace",
        "--version=15.4.0",
        "--values=-",
        "--wait",
        "--timeout=120s",
        "--",
        "shop",
        "bitnami/nginx",
    ]);
    assert_eq!(argv, expected);
    request.dry_run = true;
    let new = args::install(&target(), &request, false, HelmVersion::new(4, 3, 1));
    assert!(new.contains(&"--dry-run=server".to_owned()));
    assert!(!new.contains(&"--values=-".to_owned()));
    let old = args::install(&target(), &request, false, HelmVersion::MINIMUM);
    assert!(
        old.contains(&"--dry-run".to_owned()),
        "3.10 only has the boolean"
    );
}

#[test]
fn template_needs_no_cluster() {
    let argv = args::template(&install_request(), true);
    assert_eq!(
        argv,
        [
            "template",
            "--namespace=web",
            "--version=15.4.0",
            "--values=-",
            "--",
            "shop",
            "bitnami/nginx"
        ]
    );
}

#[test]
fn upgrade_rollback_uninstall() {
    let mut upgrade = UpgradeRequest::new(release(), ChartRef::new("nginx", "").in_repo("bitnami"));
    upgrade.reuse_values = true;
    let argv = args::upgrade(&target(), &upgrade, false, HelmVersion::new(3, 14, 0));
    assert!(argv.contains(&"--reuse-values".to_owned()));
    assert!(argv.contains(&"--timeout=300s".to_owned()));
    assert!(
        !argv.iter().any(|a| a.starts_with("--version")),
        "latest chart"
    );
    assert!(argv.ends_with(&["--".into(), "shop".into(), "bitnami/nginx".into()]));

    let mut rollback = RollbackRequest::new(release(), 2);
    rollback.dry_run = true;
    let argv = args::rollback(&target(), &rollback);
    assert!(argv.contains(&"--dry-run".to_owned()));
    assert!(argv.ends_with(&["--".into(), "shop".into(), "2".into()]));

    let mut uninstall = UninstallRequest::new(release());
    uninstall.keep_history = true;
    let argv = args::uninstall(&target(), &uninstall);
    assert_eq!(argv[0], "uninstall");
    assert!(argv.contains(&"--keep-history".to_owned()));
    assert!(argv.contains(&"--namespace=web".to_owned()));
    assert!(argv.ends_with(&["--".into(), "shop".into()]));
}

#[test]
fn repo_add_options_are_flags_and_the_password_is_never_an_argument() {
    let mut request = RepoAddRequest::new("private", "https://charts.example.com");
    request.username = Some("bob".into());
    request.password = Some(RepoPassword::new("hunter22-secret"));
    request.insecure_skip_tls_verify = true;
    request.ca_file = Some("/etc/ca.pem".into());
    let argv = args::repo_add(&request);
    assert_eq!(
        argv,
        [
            "repo",
            "add",
            "--username=bob",
            "--password-stdin",
            "--insecure-skip-tls-verify",
            "--ca-file=/etc/ca.pem",
            "--",
            "private",
            "https://charts.example.com"
        ]
    );
    assert!(!argv.join(" ").contains("hunter22"));
    request.password = Some(RepoPassword::new(""));
    assert!(!args::repo_add(&request).contains(&"--password-stdin".to_owned()));
}
