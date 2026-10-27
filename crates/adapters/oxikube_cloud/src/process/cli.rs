//! [`Cli`] (which program), [`RunOpts`] and [`CliInvocation`] (one command line and how to run
//! it), [`CliOutput`] (what a successful run printed).

use std::fmt;
use std::time::Duration;

use oxikube_domain::cloud::CloudProvider;
use tokio_util::sync::CancellationToken;

/// A command-line program this crate runs: a provider CLI or an auth plugin it checks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cli {
    /// The AWS CLI (`aws`, v2).
    Aws,
    /// The Google Cloud CLI (`gcloud`).
    Gcloud,
    /// The Azure CLI (`az`).
    Az,
    /// GKE's exec credential plugin (`gke-gcloud-auth-plugin`).
    GkeGcloudAuthPlugin,
    /// Azure's exec credential plugin (`kubelogin`).
    Kubelogin,
}

impl Cli {
    /// Every program, in a stable order.
    pub const ALL: [Self; 5] = [
        Self::Aws,
        Self::Gcloud,
        Self::Az,
        Self::GkeGcloudAuthPlugin,
        Self::Kubelogin,
    ];

    /// The provider CLI that discovers `provider`'s clusters.
    #[must_use]
    pub fn for_provider(provider: CloudProvider) -> Self {
        match provider {
            CloudProvider::Aws => Self::Aws,
            CloudProvider::Gcp => Self::Gcloud,
            CloudProvider::Azure => Self::Az,
        }
    }

    /// The program's name, as typed in a shell.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Gcloud => "gcloud",
            Self::Az => "az",
            Self::GkeGcloudAuthPlugin => "gke-gcloud-auth-plugin",
            Self::Kubelogin => "kubelogin",
        }
    }

    /// The file names to look for in each directory: the name on Unix; `.exe` and `.cmd`
    /// (how `gcloud` and `az` install) on Windows.
    pub(crate) fn file_names(self) -> Vec<String> {
        if cfg!(windows) {
            vec![
                format!("{}.exe", self.name()),
                format!("{}.cmd", self.name()),
            ]
        } else {
            vec![self.name().to_owned()]
        }
    }

    /// The flags that make the program print JSON, appended by the runner for
    /// [`OutputFormat::Json`]. The auth plugins print JSON (an `ExecCredential`) by default.
    #[must_use]
    pub fn json_flags(self) -> &'static [&'static str] {
        match self {
            Self::Aws | Self::Az => &["--output", "json"],
            Self::Gcloud => &["--format=json"],
            Self::GkeGcloudAuthPlugin | Self::Kubelogin => &[],
        }
    }

    /// The settings key that overrides where the program is (E18-S07), named in the error
    /// when it cannot be found.
    #[must_use]
    pub fn setting_key(self) -> &'static str {
        match self {
            Self::Aws => "cloud.aws_path",
            Self::Gcloud => "cloud.gcloud_path",
            Self::Az => "cloud.az_path",
            Self::GkeGcloudAuthPlugin => "cloud.gke_gcloud_auth_plugin_path",
            Self::Kubelogin => "cloud.kubelogin_path",
        }
    }

    /// What tells the user to log in again when this program says the login is missing or
    /// expired.
    #[must_use]
    pub fn login_hint(self) -> &'static str {
        match self {
            Self::Aws => "run `aws sso login` (or `aws configure`) for the profile",
            Self::Gcloud | Self::GkeGcloudAuthPlugin => "run `gcloud auth login`",
            Self::Az | Self::Kubelogin => "run `az login`",
        }
    }

    /// Environment that keeps the program non-interactive and its output plain: no pager, no
    /// prompts, no colour, no update nags.
    pub(crate) fn quiet_env(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Aws => &[("AWS_PAGER", ""), ("AWS_CLI_AUTO_PROMPT", "off")],
            Self::Gcloud | Self::GkeGcloudAuthPlugin => &[
                ("CLOUDSDK_CORE_DISABLE_PROMPTS", "1"),
                ("CLOUDSDK_COMPONENT_MANAGER_DISABLE_UPDATE_CHECK", "1"),
            ],
            Self::Az | Self::Kubelogin => &[
                ("AZURE_CORE_NO_COLOR", "1"),
                ("AZURE_CORE_ONLY_SHOW_ERRORS", "1"),
                ("AZURE_CORE_SURVEY_MESSAGE", "0"),
            ],
        }
    }
}

impl fmt::Display for Cli {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a command prints on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OutputFormat {
    /// Whatever the program prints by default.
    #[default]
    Text,
    /// JSON: the runner appends [`Cli::json_flags`].
    Json,
}

/// How to run one command.
#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    /// Kill the process after this long; `None` uses the runner's default (30 s).
    pub timeout: Option<Duration>,
    /// Cancelling it kills the process (and ends a wait for a concurrency slot).
    pub cancel: CancellationToken,
    /// Extra environment (`AWS_PROFILE`, `CLOUDSDK_CORE_PROJECT`). Never a credential: the
    /// environment is visible to other processes of the user.
    pub env: Vec<(String, String)>,
}

/// One command line: the program, its arguments, the output wanted and how to run it.
///
/// Arguments carry names (profiles, regions, projects, cluster names), never credentials:
/// command lines are visible to every process of the user.
#[derive(Debug, Clone)]
pub struct CliInvocation {
    /// The program.
    pub cli: Cli,
    /// Its arguments, subcommands first (`eks list-clusters --region eu-west-1`), without the
    /// JSON flags.
    pub args: Vec<String>,
    /// Text or JSON.
    pub format: OutputFormat,
    /// Timeout, cancellation, environment.
    pub opts: RunOpts,
}

impl CliInvocation {
    /// `cli args` with text output and default options.
    pub fn new<I, S>(cli: Cli, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            cli,
            args: args.into_iter().map(Into::into).collect(),
            format: OutputFormat::Text,
            opts: RunOpts::default(),
        }
    }

    /// The same invocation with `opts`.
    #[must_use]
    pub fn with_opts(mut self, opts: RunOpts) -> Self {
        self.opts = opts;
        self
    }

    /// The same invocation asking for JSON output.
    #[must_use]
    pub fn json(mut self) -> Self {
        self.format = OutputFormat::Json;
        self
    }

    /// The arguments the process gets: [`args`](Self::args) plus the JSON flags when asked.
    #[must_use]
    pub fn full_args(&self) -> Vec<String> {
        let mut args = self.args.clone();
        if self.format == OutputFormat::Json {
            args.extend(self.cli.json_flags().iter().map(|flag| (*flag).to_owned()));
        }
        args
    }

    /// The command's name for logs and errors: the program and its leading subcommands, up
    /// to the first flag and at most three words (`aws eks list-clusters`). Flag values and
    /// later arguments are left out.
    #[must_use]
    pub fn label(&self) -> String {
        let words = self.args.iter().take_while(|arg| !arg.starts_with('-'));
        let mut label = self.cli.name().to_owned();
        for word in words.take(3) {
            label.push(' ');
            label.push_str(word);
        }
        label
    }
}

/// What a successful run printed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliOutput {
    /// Standard output, as printed (UTF-8, invalid bytes replaced). It may hold
    /// identifiers or a credential (an auth plugin's token): parse it, never log it.
    pub stdout: String,
    /// Standard error, redacted (warnings a successful run printed).
    pub stderr: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_flags_are_appended_only_when_asked() {
        let text = CliInvocation::new(Cli::Gcloud, ["container", "clusters", "list"]);
        assert_eq!(text.full_args(), ["container", "clusters", "list"]);
        assert_eq!(
            text.json().full_args(),
            ["container", "clusters", "list", "--format=json"]
        );
        let aws = CliInvocation::new(Cli::Aws, ["eks", "list-clusters"]).json();
        assert_eq!(
            aws.full_args(),
            ["eks", "list-clusters", "--output", "json"]
        );
        let plugin = CliInvocation::new(Cli::Kubelogin, ["--version"]).json();
        assert_eq!(plugin.full_args(), ["--version"]);
    }

    #[test]
    fn labels_stop_at_the_first_flag() {
        let args = [
            "eks",
            "describe-cluster",
            "--name",
            "prod",
            "--profile",
            "admin",
        ];
        assert_eq!(
            CliInvocation::new(Cli::Aws, args).label(),
            "aws eks describe-cluster"
        );
        let args = ["--profile", "admin", "eks", "list-clusters"];
        assert_eq!(CliInvocation::new(Cli::Aws, args).label(), "aws");
        let args = ["a", "b", "c", "d"];
        assert_eq!(CliInvocation::new(Cli::Az, args).label(), "az a b c");
    }

    #[test]
    fn providers_map_to_their_cli_and_settings_keys_are_distinct() {
        assert_eq!(Cli::for_provider(CloudProvider::Gcp), Cli::Gcloud);
        assert_eq!(Cli::for_provider(CloudProvider::Azure).name(), "az");
        let mut keys: Vec<_> = Cli::ALL.iter().map(|cli| cli.setting_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Cli::ALL.len());
        assert!(Cli::Aws.login_hint().contains("aws sso login"));
    }
}
