//! `aws --version` and the per-profile login check.

use std::path::PathBuf;

use futures::future::join_all;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{CloudDetection, CloudProvider, CloudToolStatus, SetupHint};
use serde::de::IgnoredAny;

use std::collections::HashSet;

use super::{AwsDiscovery, DEFAULT_PROFILE, profile_args};
use crate::process::{Cli, CliError, CliInvocation};

/// The major version in `aws-cli/2.15.30 Python/3.11 ...`, with the full `2.15.30`.
pub(super) fn parse_version(text: &str) -> Option<(u32, String)> {
    let rest = text.split("aws-cli/").nth(1)?;
    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let major = version.split('.').next()?.parse().ok()?;
    Some((major, version))
}

impl AwsDiscovery {
    pub(super) async fn detect_cli(&self) -> OxiResult<CloudDetection> {
        let Some(binary) = self.runner.locate(Cli::Aws).await else {
            return Ok(CloudDetection::not_installed(CloudProvider::Aws));
        };
        let output = self
            .runner
            .run(CliInvocation::new(Cli::Aws, ["--version"]))
            .await?;
        // v2 prints the version on stdout, v1 on stderr.
        let text = format!("{}\n{}", output.stdout, output.stderr);
        let Some((major, version)) = parse_version(&text) else {
            return Err(OxiError::unsupported(format!(
                "`aws --version` printed something unexpected: {}",
                text.trim().chars().take(120).collect::<String>()
            )));
        };
        if major < 2 {
            return Ok(unusable(
                version,
                "AWS CLI v2 is required; this is v1. Install v2 from https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html",
            ));
        }
        self.check_logins(binary, version).await
    }

    /// Ready when at least one scanned profile has working credentials; the logged-out ones
    /// get a `aws sso login` hint.
    async fn check_logins(&self, binary: PathBuf, version: String) -> OxiResult<CloudDetection> {
        let profiles = self.profiles().await?;
        let checks = profiles.iter().map(|profile| async move {
            let mut args = vec!["sts".to_owned(), "get-caller-identity".to_owned()];
            args.extend(profile_args(profile));
            let invocation = CliInvocation::new(Cli::Aws, args);
            self.runner.run_json::<IgnoredAny>(invocation).await
        });
        let results = join_all(checks).await;
        let mut hints = Vec::new();
        let mut ready = false;
        let mut other_error: Option<CliError> = None;
        for (profile, result) in profiles.iter().zip(results) {
            match result {
                Ok(_) => ready = true,
                Err(error) if error.needs_login() => hints.push(SetupHint::with_command(
                    format!("profile `{profile}` is not logged in"),
                    format!("aws sso login --profile {profile}"),
                )),
                Err(error) => other_error = other_error.or(Some(error)),
            }
        }
        if !ready
            && hints.is_empty()
            && let Some(error) = other_error
        {
            return Err(error.into());
        }
        Ok(CloudDetection {
            provider: CloudProvider::Aws,
            status: if ready {
                CloudToolStatus::Ready
            } else {
                CloudToolStatus::NotAuthenticated
            },
            binary: Some(binary),
            version: Some(version),
            hints,
        })
    }

    /// The profiles to scan: the settings' list, else the CLI's, else just `default` (which
    /// is what environment credentials use).
    pub(super) async fn profiles(&self) -> OxiResult<Vec<String>> {
        let configured = self.settings.get().profiles;
        if !configured.is_empty() {
            return Ok(unique(configured));
        }
        let output = self
            .runner
            .run(CliInvocation::new(Cli::Aws, ["configure", "list-profiles"]))
            .await?;
        let profiles: Vec<String> = output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        let mut profiles = unique(profiles);
        if profiles.is_empty() {
            profiles.push(DEFAULT_PROFILE.to_owned());
        }
        Ok(profiles)
    }
}

/// `profiles` without repeats (adjacent or not), keeping the first occurrence's order.
fn unique(profiles: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    profiles
        .into_iter()
        .filter(|profile| seen.insert(profile.clone()))
        .collect()
}

fn unusable(version: String, summary: &str) -> CloudDetection {
    CloudDetection {
        provider: CloudProvider::Aws,
        status: CloudToolStatus::NotInstalled,
        binary: None,
        version: Some(version),
        hints: vec![SetupHint {
            summary: summary.to_owned(),
            command: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn parses_v2_and_v1_banners() {
        let v2 = "aws-cli/2.15.30 Python/3.11.8 Darwin/23.4.0 exe/arm64";
        assert_eq!(parse_version(v2), Some((2, "2.15.30".into())));
        let v1 = "aws-cli/1.27.0 Python/3.9 Linux/5 botocore/1.29";
        assert_eq!(parse_version(v1).map(|v| v.0), Some(1));
        assert_eq!(parse_version("something else"), None);
    }
}
