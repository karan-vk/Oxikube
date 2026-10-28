//! `cloud.discover_clusters`: the managed clusters cloud discovery knows, per provider.

use async_trait::async_trait;
use oxikube_domain::command::{CloudRefreshMode, Command};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    CloudProvider, DiscoveredCluster, ToolContext, ToolDef, ToolName, ToolOutput, ToolPort,
};
use serde_json::{Value, json};

use crate::cloud::CloudDiscoveryService;

/// The tool's name.
pub const DISCOVER_CLUSTERS: &str = "cloud.discover_clusters";

/// The `discover_clusters` tool: read-only, no confirmation.
pub struct DiscoverClustersTool {
    def: ToolDef,
    service: CloudDiscoveryService,
}

impl DiscoverClustersTool {
    /// The tool over `service`.
    ///
    /// # Panics
    ///
    /// Never: [`DISCOVER_CLUSTERS`] is a constant, well-formed tool name.
    pub fn new(service: CloudDiscoveryService) -> Self {
        let def = ToolDef::read_only(
            ToolName::new(DISCOVER_CLUSTERS).expect("a well-formed name"),
            "List the managed Kubernetes clusters found through a cloud CLI (aws: EKS, gcloud: \
             GKE, az: AKS): name, region, version, status and whether the cluster is already in \
             the catalog. Answers from the cache; set refresh to run the CLI once more.",
            json!({
                "type": "object",
                "properties": {
                    "provider": {
                        "type": "string",
                        "enum": ["aws", "gcloud", "az"],
                        "description": "The cloud CLI: aws, gcloud or az."
                    },
                    "refresh": {
                        "type": "boolean",
                        "description": "Run the provider's CLI again first (default false: cached clusters only)."
                    }
                },
                "required": ["provider"],
                "additionalProperties": false
            }),
        )
        .with_title("Discover Cloud Clusters");
        Self { def, service }
    }
}

impl std::fmt::Debug for DiscoverClustersTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoverClustersTool")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for DiscoverClustersTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, _: &ToolContext) -> OxiResult<ToolOutput> {
        let provider = parse_provider(args.get("provider").and_then(Value::as_str))?;
        let refresh = args
            .get("refresh")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if refresh {
            let command = Command::CloudRefresh {
                provider: Some(provider),
                mode: CloudRefreshMode::Force,
            };
            if let Err(error) = self.service.execute(&command).await {
                return Ok(ToolOutput::error(format!(
                    "Refreshing {provider} failed: {}",
                    oxikube_domain::redact::redact(error.message())
                )));
            }
        }
        let mut clusters = self.service.cached(provider).await;
        clusters.sort_by_key(DiscoveredCluster::key);
        let added: Vec<String> = self
            .service
            .added(provider)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| entry.cluster_key)
            .collect();
        let rows: Vec<Value> = clusters
            .iter()
            .map(|cluster| row(cluster, added.contains(&cluster.key())))
            .collect();
        Ok(ToolOutput::json(json!({
            "provider": provider.cli(),
            "refreshed": refresh,
            "clusters": rows,
        })))
    }
}

/// The provider named by its CLI (`aws`, `gcloud`, `az`) or its id (`gcp`, `azure`).
fn parse_provider(text: Option<&str>) -> OxiResult<CloudProvider> {
    match text {
        Some("aws") => Ok(CloudProvider::Aws),
        Some("gcloud" | "gcp") => Ok(CloudProvider::Gcp),
        Some("az" | "azure") => Ok(CloudProvider::Azure),
        other => Err(OxiError::validation(format!(
            "`provider` {other:?} is not one of aws, gcloud, az"
        ))),
    }
}

/// A cluster as a tool result: names and versions, nothing secret.
fn row(cluster: &DiscoveredCluster, in_catalog: bool) -> Value {
    json!({
        "name": cluster.name,
        "region": cluster.location,
        "version": cluster.kubernetes_version,
        "status": cluster.status.to_string(),
        "in_catalog": in_catalog,
    })
}
