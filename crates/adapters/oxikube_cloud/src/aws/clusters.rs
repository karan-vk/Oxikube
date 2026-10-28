//! `eks list-clusters` and `eks describe-cluster`, and their JSON parsers.
//!
//! The parsers ignore unknown fields (CLI drift). The CLI follows `nextToken` itself unless
//! given `--no-paginate`, so `list-clusters` is complete without us passing tokens.

use futures::future::join_all;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{CloudScope, ClusterStatus, DiscoveredCluster};
use serde::Deserialize;

use super::{AwsDiscovery, profile_args};
use crate::process::{Cli, CliError, CliInvocation};

#[derive(Deserialize)]
struct ListClusters {
    #[serde(default)]
    clusters: Vec<String>,
}

#[derive(Deserialize)]
struct DescribeCluster {
    cluster: EksCluster,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EksCluster {
    name: String,
    arn: Option<String>,
    endpoint: Option<String>,
    version: Option<String>,
    status: Option<String>,
    certificate_authority: Option<CertificateAuthority>,
}

#[derive(Deserialize)]
struct CertificateAuthority {
    data: Option<String>,
}

fn status(raw: Option<&str>) -> ClusterStatus {
    match raw {
        Some("ACTIVE") => ClusterStatus::Running,
        Some("CREATING") => ClusterStatus::Creating,
        Some("UPDATING") => ClusterStatus::Updating,
        Some("DELETING") => ClusterStatus::Deleting,
        Some("FAILED") => ClusterStatus::Failed,
        Some(other) => ClusterStatus::Other(other.to_owned()),
        None => ClusterStatus::Other("UNKNOWN".to_owned()),
    }
}

impl EksCluster {
    fn into_discovered(self, scope: &CloudScope, region: &str) -> DiscoveredCluster {
        DiscoveredCluster {
            native_id: self.arn,
            endpoint: self.endpoint.filter(|e| !e.is_empty()),
            ca_data: self
                .certificate_authority
                .and_then(|ca| ca.data)
                .filter(|d| !d.is_empty()),
            kubernetes_version: self.version,
            status: status(self.status.as_deref()),
            ..DiscoveredCluster::new(scope.clone(), self.name, region)
        }
    }
}

impl AwsDiscovery {
    pub(super) async fn list_eks(&self, scope: &CloudScope) -> OxiResult<Vec<DiscoveredCluster>> {
        let CloudScope::Aws { profile, region } = scope else {
            return Err(OxiError::validation(format!(
                "the AWS adapter cannot list the {} scope `{scope}`",
                scope.provider()
            )));
        };
        let list: ListClusters = self
            .runner
            .run_json(CliInvocation::new(Cli::Aws, {
                let mut args = vec!["eks", "list-clusters", "--region", region]
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                args.extend(profile_args(profile));
                args
            }))
            .await?;
        // The runner's concurrency cap bounds how many describes run at once.
        let described = join_all(list.clusters.iter().map(|name| async move {
            self.runner
                .run_json::<DescribeCluster>(CliInvocation::new(Cli::Aws, {
                    let mut args = [
                        "eks",
                        "describe-cluster",
                        "--name",
                        name,
                        "--region",
                        region,
                    ]
                    .map(str::to_owned)
                    .to_vec();
                    args.extend(profile_args(profile));
                    args
                }))
                .await
        }))
        .await;
        let mut clusters = Vec::with_capacity(described.len());
        for result in described {
            match result {
                Ok(described) => clusters.push(described.cluster.into_discovered(scope, region)),
                // Deleted between the list and the describe.
                Err(error) if is_gone(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }
        clusters.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(clusters)
    }
}

fn is_gone(error: &CliError) -> bool {
    error.says("ResourceNotFoundException")
}
