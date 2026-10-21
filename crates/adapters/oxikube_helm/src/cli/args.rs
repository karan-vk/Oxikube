//! The argv of every `helm` command the adapter runs. Pure functions, tested without a binary.
//!
//! # No shell, no injection, no values
//!
//! The argv is passed to the process directly, never through a shell. Flags are written as
//! `--flag=value`, so a value starting with `-` is never read as another flag, and every
//! positional (release, chart, repository names) comes after `--`, so a name starting with `-`
//! is a name. Values are never on the command line (it is visible in `ps`): when a request has
//! any, the argv carries `--values=-` and the YAML goes to stdin. Flags are the ones Helm 3.10
//! and 4.x both accept (Helm 4 dropped `list --all`, so the status filters are spelled out).

use oxikube_domain::helm::{
    ChartRef, HelmVersion, InstallRequest, RepoAddRequest, RollbackRequest, SearchQuery, ShowKind,
    UninstallRequest, UpgradeRequest,
};
use std::time::Duration;

use super::target::HelmTarget;

/// The first Helm that takes `--dry-run=server` (3.10–3.12 only know the boolean form).
const DRY_RUN_MODES: HelmVersion = HelmVersion::new(3, 13, 0);

/// `list` filters equivalent to Helm 3's `--all`.
const ALL_STATES: &[&str] = &[
    "--deployed",
    "--failed",
    "--pending",
    "--superseded",
    "--uninstalled",
    "--uninstalling",
];

/// An argv under construction: sub-command words, flags, then `--` and the positionals.
#[derive(Debug, Default)]
struct Argv {
    words: Vec<String>,
    flags: Vec<String>,
    positionals: Vec<String>,
}

impl Argv {
    fn new(words: &[&str]) -> Self {
        Self {
            words: words.iter().map(|w| (*w).to_owned()).collect(),
            ..Self::default()
        }
    }

    fn flag(mut self, flag: &str) -> Self {
        self.flags.push(flag.to_owned());
        self
    }

    fn flag_if(self, on: bool, flag: &str) -> Self {
        if on { self.flag(flag) } else { self }
    }

    fn opt(mut self, name: &str, value: impl std::fmt::Display) -> Self {
        self.flags.push(format!("--{name}={value}"));
        self
    }

    fn target(mut self, target: &HelmTarget) -> Self {
        self.flags.extend(target.args());
        self
    }

    fn namespace(self, namespace: &str) -> Self {
        self.opt("namespace", namespace)
    }

    fn json(self) -> Self {
        self.opt("output", "json")
    }

    fn timeout(self, timeout: Duration) -> Self {
        self.opt("timeout", format!("{}s", timeout.as_secs().max(1)))
    }

    fn version(self, chart: &ChartRef) -> Self {
        if chart.version.is_empty() {
            self
        } else {
            self.opt("version", &chart.version)
        }
    }

    fn values(self, has_values: bool) -> Self {
        self.flag_if(has_values, "--values=-")
    }

    fn arg(mut self, positional: impl Into<String>) -> Self {
        self.positionals.push(positional.into());
        self
    }

    fn build(self) -> Vec<String> {
        let mut argv = self.words;
        argv.extend(self.flags);
        if !self.positionals.is_empty() {
            argv.push("--".to_owned());
            argv.extend(self.positionals);
        }
        argv
    }
}

/// `helm list`: every release's latest revision, in `namespace` or (when `None`) all of them.
pub(crate) fn list(target: &HelmTarget, namespace: Option<&str>) -> Vec<String> {
    let argv = Argv::new(&["list"]).target(target).json().opt("max", 0);
    let argv = ALL_STATES.iter().fold(argv, |argv, state| argv.flag(state));
    match namespace {
        Some(namespace) => argv.namespace(namespace),
        None => argv.flag("--all-namespaces"),
    }
    .build()
}

/// `helm history <release>`.
pub(crate) fn history(target: &HelmTarget, namespace: &str, release: &str) -> Vec<String> {
    Argv::new(&["history"])
        .target(target)
        .namespace(namespace)
        .json()
        .arg(release)
        .build()
}

/// What `helm get` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Get {
    /// `get values` (`--all` for the computed values).
    Values {
        /// Chart defaults merged in.
        all: bool,
    },
    /// `get manifest`.
    Manifest,
    /// `get notes`.
    Notes,
}

/// `helm get values|manifest|notes <release> [--revision N]`.
pub(crate) fn get(
    target: &HelmTarget,
    namespace: &str,
    release: &str,
    revision: Option<u32>,
    what: Get,
) -> Vec<String> {
    let word = match what {
        Get::Values { .. } => "values",
        Get::Manifest => "manifest",
        Get::Notes => "notes",
    };
    let mut argv = Argv::new(&["get", word])
        .target(target)
        .namespace(namespace);
    if let Get::Values { all } = what {
        argv = argv.json().flag_if(all, "--all");
    }
    if let Some(revision) = revision {
        argv = argv.opt("revision", revision);
    }
    argv.arg(release).build()
}

/// `helm repo list`.
pub(crate) fn repo_list() -> Vec<String> {
    Argv::new(&["repo", "list"]).json().build()
}

/// `helm repo add <name> <url>`. A password is never an argument: with one, `--password-stdin`
/// makes `helm` read it from stdin.
pub(crate) fn repo_add(request: &RepoAddRequest) -> Vec<String> {
    let mut argv = Argv::new(&["repo", "add"]);
    if let Some(username) = request.username.as_deref().filter(|u| !u.is_empty()) {
        argv = argv.opt("username", username);
    }
    let has_password = request.password.as_ref().is_some_and(|p| !p.is_empty());
    argv = argv.flag_if(has_password, "--password-stdin").flag_if(
        request.insecure_skip_tls_verify,
        "--insecure-skip-tls-verify",
    );
    if let Some(ca) = request.ca_file.as_deref().filter(|c| !c.is_empty()) {
        argv = argv.opt("ca-file", ca);
    }
    argv.arg(request.name.as_str())
        .arg(request.url.as_str())
        .build()
}

/// `helm repo remove <name>`.
pub(crate) fn repo_remove(name: &str) -> Vec<String> {
    Argv::new(&["repo", "remove"]).arg(name).build()
}

/// `helm repo update [names...]` (all repositories when `names` is empty).
pub(crate) fn repo_update(names: &[String]) -> Vec<String> {
    names
        .iter()
        .fold(Argv::new(&["repo", "update"]), |argv, name| {
            argv.arg(name.as_str())
        })
        .build()
}

/// `helm search repo [keyword] [--versions]`.
pub(crate) fn search(query: &SearchQuery) -> Vec<String> {
    let argv = Argv::new(&["search", "repo"])
        .json()
        .flag_if(query.all_versions, "--versions");
    match query.keyword.trim() {
        "" => argv.build(),
        keyword => argv.arg(keyword).build(),
    }
}

/// `helm show <kind> <chart> [--version v]`.
pub(crate) fn show(chart: &ChartRef, kind: ShowKind) -> Vec<String> {
    Argv::new(&["show", kind.as_str()])
        .version(chart)
        .arg(chart.reference())
        .build()
}

/// `helm template <release> <chart>`: local rendering, so no cluster flags.
pub(crate) fn template(request: &InstallRequest, has_values: bool) -> Vec<String> {
    Argv::new(&["template"])
        .namespace(&request.namespace)
        .version(&request.chart)
        .values(has_values)
        .arg(request.release.as_str())
        .arg(request.chart.reference())
        .build()
}

fn dry_run(argv: Argv, on: bool, version: HelmVersion) -> Argv {
    match (on, version >= DRY_RUN_MODES) {
        (false, _) => argv,
        (true, true) => argv.flag("--dry-run=server"),
        (true, false) => argv.flag("--dry-run"),
    }
}

/// `helm install <release> <chart>`.
pub(crate) fn install(
    target: &HelmTarget,
    request: &InstallRequest,
    has_values: bool,
    version: HelmVersion,
) -> Vec<String> {
    let argv = Argv::new(&["install"])
        .target(target)
        .namespace(&request.namespace)
        .flag_if(request.create_namespace, "--create-namespace")
        .version(&request.chart)
        .values(has_values)
        .flag_if(request.wait, "--wait")
        .timeout(request.timeout);
    dry_run(argv, request.dry_run, version)
        .arg(request.release.as_str())
        .arg(request.chart.reference())
        .build()
}

/// `helm upgrade <release> <chart>`.
pub(crate) fn upgrade(
    target: &HelmTarget,
    request: &UpgradeRequest,
    has_values: bool,
    version: HelmVersion,
) -> Vec<String> {
    let argv = Argv::new(&["upgrade"])
        .target(target)
        .namespace(&request.release.namespace)
        .version(&request.chart)
        .values(has_values)
        .flag_if(request.reuse_values, "--reuse-values")
        .flag_if(request.wait, "--wait")
        .timeout(request.timeout);
    dry_run(argv, request.dry_run, version)
        .arg(request.release.name.as_str())
        .arg(request.chart.reference())
        .build()
}

/// `helm rollback <release> <revision>`.
pub(crate) fn rollback(target: &HelmTarget, request: &RollbackRequest) -> Vec<String> {
    Argv::new(&["rollback"])
        .target(target)
        .namespace(&request.release.namespace)
        .flag_if(request.wait, "--wait")
        .flag_if(request.dry_run, "--dry-run")
        .timeout(request.timeout)
        .arg(request.release.name.as_str())
        .arg(request.revision.to_string())
        .build()
}

/// `helm uninstall <release>`.
pub(crate) fn uninstall(target: &HelmTarget, request: &UninstallRequest) -> Vec<String> {
    Argv::new(&["uninstall"])
        .target(target)
        .namespace(&request.release.namespace)
        .flag_if(request.keep_history, "--keep-history")
        .flag_if(request.dry_run, "--dry-run")
        .timeout(request.timeout)
        .arg(request.release.name.as_str())
        .build()
}
