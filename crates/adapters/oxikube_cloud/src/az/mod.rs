//! [`AzureAksAdapter`]: AKS discovery through the `az` CLI (E18-S04).
//!
//! | File | Holds |
//! |---|---|
//! | `model.rs` | the `az` JSON shapes and their mapping onto the domain types |
//! | `credentials.rs` | `az aks get-credentials` into a private temp dir, the server and CA read out of it, the file deleted |
//! | `entry.rs` | the `kubelogin` exec kubeconfig entry for an Azure AD cluster |
//!
//! # Flow
//!
//! `detect` runs `az version` and `az account show` (not logged in is
//! [`NotAuthenticated`](CloudToolStatus::NotAuthenticated) with the `az login` hint) and notes a
//! missing `kubelogin`; `scopes` lists the enabled subscriptions (`az account list`);
//! `list_clusters` runs `az aks list --subscription S`; `kubeconfig_entry` runs `az aks show`
//! (is it Azure AD? stopped?) and, when the listing lacked the CA, `az aks get-credentials` into
//! a temporary file.
//!
//! # Security
//!
//! `get-credentials` can write client certificates or tokens. The file lives in a `0700`
//! directory that is deleted whether the call succeeds, fails or is dropped, only the cluster's
//! `server` and `certificate-authority-data` (public data) are read from it, and it is never
//! logged. The produced entry authenticates only through `kubelogin get-token`, so tokens are
//! fetched when connecting and never stored. `--admin` is never passed.

mod credentials;
mod entry;
mod model;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::cloud::SetupHint;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    CloudDetection, CloudDiscoveryPort, CloudProvider, CloudScope, CloudToolStatus,
    DiscoveredCluster, KubeconfigEntry,
};
use serde::de::IgnoredAny;

use crate::process::{Cli, CliInvocation, ProcessRunner, RunOpts};
use crate::shared::Shared;
use model::{AksCluster, AzVersion, Subscription};

/// How long listing a subscription's clusters may take (`az` starts slowly and a subscription
/// can hold many clusters).
const LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// Where users install `kubelogin` from.
const KUBELOGIN_INSTALL_URL: &str = "https://azure.github.io/kubelogin/install.html";

/// Discovers AKS clusters per Azure subscription through `az` and builds `kubelogin` exec
/// entries for the Azure AD ones.
pub struct AzureAksAdapter {
    runner: Arc<dyn ProcessRunner>,
    subscriptions: Shared<Vec<String>>,
}

impl AzureAksAdapter {
    /// An adapter running `az` (and locating `kubelogin`) through `runner`.
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self {
            runner,
            subscriptions: Shared::new(Vec::new()),
        }
    }

    /// Scans only the subscriptions whose id or display name is in `subscriptions` (the
    /// `cloud.az.subscriptions` setting); empty scans every enabled one. The next scan uses it.
    pub fn set_subscriptions(&self, subscriptions: Vec<String>) {
        self.subscriptions.set(subscriptions);
    }

    fn list_opts() -> RunOpts {
        RunOpts {
            timeout: Some(LIST_TIMEOUT),
            ..RunOpts::default()
        }
    }

    fn azure_subscription(scope: &CloudScope) -> OxiResult<&str> {
        match scope {
            CloudScope::Azure {
                subscription_id, ..
            } => Ok(subscription_id),
            CloudScope::Aws { .. } | CloudScope::Gcp { .. } => Err(OxiError::validation(format!(
                "the Azure adapter cannot list the {} scope `{scope}`",
                scope.provider()
            ))),
        }
    }
}

#[async_trait]
impl CloudDiscoveryPort for AzureAksAdapter {
    fn id(&self) -> CloudProvider {
        CloudProvider::Azure
    }

    async fn detect(&self) -> OxiResult<CloudDetection> {
        let Some(binary) = self.runner.locate(Cli::Az).await else {
            return Ok(CloudDetection::not_installed(CloudProvider::Azure));
        };
        let version: AzVersion = self
            .runner
            .run_json(CliInvocation::new(Cli::Az, ["version"]))
            .await?;
        let mut hints = Vec::new();
        let account = self
            .runner
            .run_json::<IgnoredAny>(CliInvocation::new(Cli::Az, ["account", "show"]))
            .await;
        let status = match account {
            Ok(_) => CloudToolStatus::Ready,
            Err(error) if error.needs_login() => {
                hints.push(SetupHint::with_command(
                    "the Azure CLI is not logged in",
                    "az login",
                ));
                CloudToolStatus::NotAuthenticated
            }
            Err(error) => return Err(error.into()),
        };
        if self.runner.locate(Cli::Kubelogin).await.is_none() {
            hints.push(SetupHint::with_command(
                "kubelogin is not installed; clusters with Azure AD sign-in need it",
                "az aks install-cli",
            ));
        }
        Ok(CloudDetection {
            provider: CloudProvider::Azure,
            status,
            binary: Some(binary),
            version: version.azure_cli,
            hints,
        })
    }

    async fn scopes(&self) -> OxiResult<Vec<CloudScope>> {
        let subscriptions: Vec<Subscription> = self
            .runner
            .run_json(CliInvocation::new(Cli::Az, ["account", "list"]))
            .await?;
        let wanted = self.subscriptions.get();
        Ok(subscriptions
            .into_iter()
            .filter(Subscription::is_enabled)
            .filter(|subscription| wanted.is_empty() || subscription.matches_any(&wanted))
            .map(Subscription::into_scope)
            .collect())
    }

    async fn list_clusters(&self, scope: &CloudScope) -> OxiResult<Vec<DiscoveredCluster>> {
        let subscription = Self::azure_subscription(scope)?;
        let invocation =
            CliInvocation::new(Cli::Az, ["aks", "list", "--subscription", subscription])
                .with_opts(Self::list_opts());
        let clusters: Vec<AksCluster> = self.runner.run_json(invocation).await?;
        Ok(clusters
            .into_iter()
            .map(|cluster| cluster.into_discovered(scope))
            .collect())
    }

    async fn kubeconfig_entry(&self, cluster: &DiscoveredCluster) -> OxiResult<KubeconfigEntry> {
        entry::build(self.runner.as_ref(), cluster).await
    }
}
