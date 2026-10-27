//! Detecting `gcloud`: version, login, Application Default Credentials, the auth plugin; and
//! listing projects.

use std::collections::BTreeMap;
use std::path::PathBuf;

use oxikube_domain::OxiResult;
use oxikube_ports::cloud::{CloudDetection, CloudProvider, CloudScope, CloudToolStatus, SetupHint};

use super::parse::{Account, Project};
use super::{GcloudDiscovery, PLUGIN_COMMAND, PLUGIN_INSTALL};
use crate::process::{Cli, CliError, CliInvocation};

impl GcloudDiscovery {
    pub(super) async fn detect_gcloud(&self) -> OxiResult<CloudDetection> {
        let Some(binary) = self.runner.locate(Cli::Gcloud).await else {
            return Ok(CloudDetection::not_installed(CloudProvider::Gcp));
        };
        let version = match self
            .runner
            .run_json::<BTreeMap<String, serde_json::Value>>(self.json(["version"]))
            .await
        {
            Ok(map) => map
                .get("Google Cloud SDK")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            Err(CliError::NotFound { .. }) => {
                return Ok(CloudDetection::not_installed(CloudProvider::Gcp));
            }
            Err(error) => return Err(error.into()),
        };
        let accounts: Vec<Account> = self.runner.run_json(self.json(["auth", "list"])).await?;
        let has_account = accounts
            .iter()
            .any(|a| a.status == "ACTIVE" && !a.account.is_empty());

        let mut hints = Vec::new();
        if has_account {
            if !adc_exists() {
                hints.push(SetupHint::with_command(
                    "no Application Default Credentials: the GKE auth plugin cannot sign in",
                    "gcloud auth application-default login",
                ));
            }
        } else {
            hints.push(SetupHint::with_command(
                "gcloud has no active account",
                "gcloud auth login",
            ));
        }
        if self.runner.locate(Cli::GkeGcloudAuthPlugin).await.is_none() {
            hints.push(SetupHint::with_command(
                format!("{PLUGIN_COMMAND} is not installed: GKE clusters cannot be connected"),
                PLUGIN_INSTALL,
            ));
        }
        Ok(CloudDetection {
            provider: CloudProvider::Gcp,
            status: if has_account {
                CloudToolStatus::Ready
            } else {
                CloudToolStatus::NotAuthenticated
            },
            binary: Some(binary),
            version,
            hints,
        })
    }

    /// Active projects, the `gcloud config` project first, then by id.
    pub(super) async fn list_projects(&self) -> OxiResult<Vec<CloudScope>> {
        let projects: Vec<Project> = self
            .runner
            .run_json(self.json(["projects", "list"]))
            .await?;
        let mut ids: Vec<String> = projects
            .into_iter()
            .filter(Project::is_usable)
            .map(|p| p.project_id)
            .collect();
        ids.sort();
        ids.dedup();
        if let Some(active) = self.active_project().await
            && let Some(at) = ids.iter().position(|id| *id == active)
        {
            let id = ids.remove(at);
            ids.insert(0, id);
        }
        Ok(ids
            .into_iter()
            .map(|project| CloudScope::Gcp { project })
            .collect())
    }

    /// The project `gcloud config` points at; `None` when unset or unreadable.
    async fn active_project(&self) -> Option<String> {
        let invocation = CliInvocation::new(Cli::Gcloud, ["config", "get-value", "project"]);
        let output = self.runner.run(invocation).await.ok()?;
        let value = output.stdout.trim();
        (!value.is_empty() && value != "(unset)").then(|| value.to_owned())
    }
}

/// Application Default Credentials exist: `GOOGLE_APPLICATION_CREDENTIALS` names a file, or
/// the well-known file is in the gcloud config folder. Only the path is checked; the file is
/// never opened.
fn adc_exists() -> bool {
    if let Some(path) = std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS") {
        return PathBuf::from(path).is_file();
    }
    let config = std::env::var_os("CLOUDSDK_CONFIG")
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(windows) {
                std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("gcloud"))
            } else {
                std::env::var_os("HOME").map(|dir| PathBuf::from(dir).join(".config/gcloud"))
            }
        });
    config.is_some_and(|dir| dir.join("application_default_credentials.json").is_file())
}
