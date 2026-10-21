//! Chart repositories, chart search and `helm show`.

/// A configured chart repository (`helm repo list`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoEntry {
    /// The alias (`bitnami`).
    pub name: String,
    /// The index URL.
    pub url: String,
}

/// One chart version found by a search (`helm search repo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartHit {
    /// The chart, with its repository and this version.
    pub chart: super::release::ChartRef,
    /// The application version this chart version packages.
    pub app_version: Option<String>,
    /// The chart's one-line description.
    pub description: String,
}

/// What to search the configured repositories for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    /// Keyword; empty lists every chart.
    pub keyword: String,
    /// Every version of each chart (`--versions`) instead of only the latest.
    pub all_versions: bool,
}

/// Which part of a chart `helm show` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShowKind {
    /// `Chart.yaml`.
    Chart,
    /// The README, markdown.
    Readme,
    /// The default `values.yaml`.
    Values,
    /// The custom resource definitions the chart ships.
    Crds,
    /// All of the above.
    All,
}

impl ShowKind {
    /// The `helm show` sub-command.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chart => "chart",
            Self::Readme => "readme",
            Self::Values => "values",
            Self::Crds => "crds",
            Self::All => "all",
        }
    }
}

/// A repository password or token. It is kept in memory only: never serialised, and its
/// `Debug` output is blanked, so it cannot reach a log, an audit record or a snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct RepoPassword(String);

impl RepoPassword {
    /// Wraps `secret`.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The secret, for the one place that hands it to `helm` (its stdin).
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// `true` when nothing was typed.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for RepoPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RepoPassword([redacted])")
    }
}

/// A repository to add (`helm repo add`): its alias and URL, and how to reach it.
///
/// Helm itself stores what is given here (including the password) in its own
/// `repositories.yaml`; Oxikube keeps no copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoAddRequest {
    /// The alias.
    pub name: String,
    /// The index URL; it must not carry credentials.
    pub url: String,
    /// Basic-auth user name.
    pub username: Option<String>,
    /// Basic-auth password or token.
    pub password: Option<RepoPassword>,
    /// Do not verify the server's TLS certificate (`--insecure-skip-tls-verify`).
    pub insecure_skip_tls_verify: bool,
    /// A CA bundle to verify the server with (`--ca-file`).
    pub ca_file: Option<String>,
}

impl RepoAddRequest {
    /// A plain, anonymous repository.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            username: None,
            password: None,
            insecure_skip_tls_verify: false,
            ca_file: None,
        }
    }
}

/// Why a repository alias or URL cannot be used, spelled for the person typing it.
///
/// # Errors
///
/// A message when `name` is empty or has characters Helm does not allow in an alias, or
/// `url` is not an `http(s)` URL (Helm's `repo add` takes index repositories only; OCI registries
/// are installed from directly, E29).
pub fn validate_repo(name: &str, url: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("Give the repository a name.");
    }
    if name.starts_with('-')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("A name uses letters, digits, '-', '_' and '.', and does not start with '-'.");
    }
    let url = url.trim();
    if url.is_empty() {
        return Err("Enter the repository URL.");
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("The URL starts with https:// (or http://).");
    }
    Ok(())
}
