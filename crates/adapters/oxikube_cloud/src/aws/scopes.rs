//! Profiles x regions: what to scan.

use futures::future::join_all;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::CloudScope;

use super::AwsDiscovery;
use crate::process::{Cli, CliInvocation};

impl AwsDiscovery {
    /// One scope per profile and region: the settings' regions plus the profile's own default
    /// region. Nothing to scan at all is an error naming the fix, not an empty success.
    pub(super) async fn scan_scopes(&self) -> OxiResult<Vec<CloudScope>> {
        let profiles = self.profiles().await?;
        let defaults = join_all(profiles.iter().map(|p| self.default_region(p))).await;
        let mut scopes = Vec::new();
        for (profile, default) in profiles.into_iter().zip(defaults) {
            let mut regions = self.settings.regions.clone();
            regions.extend(default);
            regions.sort();
            regions.dedup();
            scopes.extend(regions.into_iter().map(|region| CloudScope::Aws {
                profile: profile.clone(),
                region,
            }));
        }
        if scopes.is_empty() {
            return Err(OxiError::validation(
                "no AWS regions to scan: add regions to the cloud.aws settings or set a default \
                 region with `aws configure set region <region> --profile <profile>`",
            ));
        }
        Ok(scopes)
    }

    /// `aws configure get region --profile P`, which exits 1 with no output when unset (or
    /// the call fails): either way the profile just has no default region.
    async fn default_region(&self, profile: &str) -> Option<String> {
        let invocation = CliInvocation::new(
            Cli::Aws,
            ["configure", "get", "region", "--profile", profile],
        );
        let output = self.runner.run(invocation).await.ok()?;
        let region = output.stdout.trim();
        (!region.is_empty()).then(|| region.to_owned())
    }
}
