//! `az aks get-credentials` into a private temporary file, reduced to what is public.
//!
//! The kubeconfig `az` writes can hold a client certificate, a client key or a token. Only the
//! cluster's `server` and `certificate-authority-data` are read from it; the user section is
//! never deserialised, kept or logged, and the directory holding the file (`0700`, created by
//! `tempfile`) is removed before the function returns. Dropping the future (cancellation) drops
//! the directory too.

#[cfg(unix)]
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use oxikube_domain::{OxiError, OxiResult};
use serde::Deserialize;
use tempfile::TempDir;

use crate::process::{Cli, CliInvocation, ProcessRunner};

/// What a cluster's kubeconfig says about the cluster itself: public data.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ClusterInfo {
    /// The API server URL.
    pub server: String,
    /// `certificate-authority-data`, base64 PEM.
    pub ca_data: Option<String>,
}

#[derive(Deserialize)]
struct Kubeconfig {
    #[serde(default)]
    clusters: Vec<NamedCluster>,
}

#[derive(Deserialize)]
struct NamedCluster {
    cluster: ClusterBody,
}

#[derive(Deserialize)]
struct ClusterBody {
    server: String,
    #[serde(rename = "certificate-authority-data")]
    ca_data: Option<String>,
}

/// Reads the first cluster of kubeconfig `yaml`. Parse errors are not echoed: the text can
/// hold credentials.
pub(super) fn parse_cluster_info(yaml: &str) -> OxiResult<ClusterInfo> {
    let config: Kubeconfig = serde_saphyr::from_str(yaml).map_err(|_| {
        OxiError::internal(
            "the kubeconfig `az aks get-credentials` wrote is not in the expected format",
        )
    })?;
    let cluster = config
        .clusters
        .into_iter()
        .next()
        .ok_or_else(|| OxiError::internal("`az aks get-credentials` wrote no cluster"))?;
    Ok(ClusterInfo {
        server: cluster.cluster.server,
        ca_data: cluster.cluster.ca_data.filter(|data| !data.is_empty()),
    })
}

/// A fresh temporary directory only the user can enter (`0700`), set explicitly rather than
/// trusting the platform default.
fn private_dir() -> io::Result<TempDir> {
    let dir = tempfile::Builder::new().prefix("oxikube-aks-").tempdir()?;
    #[cfg(unix)]
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// Runs `az aks get-credentials` for the cluster into a fresh private directory, reads the
/// cluster's endpoint and CA, and deletes the directory.
pub(super) async fn fetch_cluster_info(
    runner: &dyn ProcessRunner,
    subscription: &str,
    resource_group: &str,
    name: &str,
) -> OxiResult<ClusterInfo> {
    let dir = private_dir()
        .map_err(|error| OxiError::internal(format!("no private temp directory: {error}")))?;
    let file = dir.path().join("kubeconfig");
    let invocation = CliInvocation::new(
        Cli::Az,
        [
            "aks",
            "get-credentials",
            "--name",
            name,
            "--resource-group",
            resource_group,
            "--subscription",
            subscription,
            "--file",
            &file.to_string_lossy(),
            "--overwrite-existing",
        ],
    );
    let outcome = async {
        runner.run(invocation).await?;
        let text = tokio::fs::read_to_string(&file).await.map_err(|error| {
            OxiError::internal(format!("`az aks get-credentials` wrote no file: {error}"))
        })?;
        parse_cluster_info(&text)
    }
    .await;
    if let Err(error) = dir.close() {
        // Not fatal for the caller, but a leftover file is worth knowing about. Only the
        // OS error is logged, never the file's content.
        tracing::warn!(%error, "cloud: could not delete the temporary AKS kubeconfig directory");
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    const KUBECONFIG: &str = "\
apiVersion: v1
kind: Config
clusters:
- name: prod
  cluster:
    server: https://prod-dns-0000.hcp.westeurope.azmk8s.io:443
    certificate-authority-data: LS0tLS1CRUdJTkNB
contexts:
- name: prod
  context: {cluster: prod, user: clusterUser_rg_prod}
users:
- name: clusterUser_rg_prod
  user:
    client-certificate-data: SECRETCERT
    client-key-data: SECRETKEY
";

    #[test]
    fn reads_server_and_ca_only() {
        let info = parse_cluster_info(KUBECONFIG).unwrap();
        assert_eq!(
            info,
            ClusterInfo {
                server: "https://prod-dns-0000.hcp.westeurope.azmk8s.io:443".into(),
                ca_data: Some("LS0tLS1CRUdJTkNB".into()),
            }
        );
        assert!(!format!("{info:?}").contains("SECRET"));
    }

    #[test]
    fn bad_files_do_not_echo_their_content() {
        let error = parse_cluster_info("clusters: [token: SECRETTOKEN").unwrap_err();
        assert!(!error.to_string().contains("SECRETTOKEN"), "{error}");
        let empty = parse_cluster_info("clusters: []").unwrap_err();
        assert!(empty.to_string().contains("no cluster"));
    }
}
