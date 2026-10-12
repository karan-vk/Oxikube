//! `k8s.get_secret` (E12-S11): a read tool for one Secret that returns it **masked**: the type,
//! the key names and sizes, and the type's public facts (registries and usernames of a registry
//! login, the subject and expiry of a TLS certificate), never a value.
//!
//! Decoded values are returned only when the call passes `reveal: true` **and** the tool's
//! [`RevealPolicy`] allows it for that caller. The policy defaults to [`DenyReveal`]: until the
//! agent epic's permission prompt (E26) can ask the user, no agent can read a Secret's values
//! through this tool, and a call that asks is a tool error the model sees, not a silent mask.
//! Binary values are never returned, only their size. Nothing the tool returns is logged by it.
//!
//! | Argument | Meaning |
//! |---|---|
//! | `name` | the Secret's name (required) |
//! | `namespace` | default `default` |
//! | `reveal` | `true` asks for decoded text values; refused unless the policy allows it |

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::secret::SecretError;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{
    ResourceReader, ToolAnnotations, ToolContext, ToolDef, ToolName, ToolOutput, ToolPort,
};
use serde_json::{Map, Value, json};

use crate::secrets::{SecretService, SecretSummary, TypeSummary};
use crate::session::ClusterSessionManager;
use crate::tools::{RegisterToolError, ToolRegistry};

/// The tool's name.
pub const GET_SECRET: &str = "k8s.get_secret";

/// Who may read decoded values through the tool.
pub trait RevealPolicy: Send + Sync {
    /// Whether the caller described by `ctx` may be given the values of Secrets.
    fn allows(&self, ctx: &ToolContext) -> bool;
}

/// The default policy: nobody. Values are never returned.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyReveal;

impl RevealPolicy for DenyReveal {
    fn allows(&self, _: &ToolContext) -> bool {
        false
    }
}

/// Resolves the cluster a call is about and the reader of its objects. Implemented by
/// [`ClusterSessionManager`]; tests pass a fixed reader.
pub trait SecretClusters: Send + Sync {
    /// The cluster and reader for `cluster`, or the only connected cluster when `None`.
    ///
    /// # Errors
    ///
    /// `NotFound` for a cluster with no session, `Network` for one that is not connected, and a
    /// validation error when none is named and several (or no) clusters are connected.
    fn reader(
        &self,
        cluster: Option<&ClusterId>,
    ) -> OxiResult<(ClusterId, Arc<dyn ResourceReader>)>;
}

impl SecretClusters for ClusterSessionManager {
    fn reader(
        &self,
        cluster: Option<&ClusterId>,
    ) -> OxiResult<(ClusterId, Arc<dyn ResourceReader>)> {
        let session = match cluster {
            Some(id) => self
                .get(id)
                .ok_or_else(|| OxiError::not_found(format!("no session for cluster {id}")))?,
            None => {
                let mut connected = self.sessions().into_iter().filter(|s| s.is_connected());
                match (connected.next(), connected.next()) {
                    (Some(only), None) => only,
                    (None, _) => return Err(OxiError::network("no cluster is connected")),
                    (Some(_), Some(_)) => {
                        return Err(OxiError::validation(
                            "several clusters are connected: name the cluster",
                        ));
                    }
                }
            }
        };
        let reader = session.resources().ok_or_else(|| {
            OxiError::network(format!("cluster {} is not connected", session.title()))
        })?;
        Ok((session.id().clone(), reader))
    }
}

/// The `k8s.get_secret` tool: see the [module docs](self).
pub struct GetSecretTool {
    def: ToolDef,
    clusters: Arc<dyn SecretClusters>,
    policy: Arc<dyn RevealPolicy>,
    service: SecretService,
}

impl GetSecretTool {
    /// The tool, reading through `clusters`, revealing values only where `policy` allows.
    pub fn new(clusters: Arc<dyn SecretClusters>, policy: Arc<dyn RevealPolicy>) -> Self {
        Self {
            def: definition(),
            clusters,
            policy,
            service: SecretService::new(),
        }
    }

    /// Registers the tool on `registry`, with the default [`DenyReveal`] policy.
    ///
    /// # Errors
    ///
    /// [`RegisterToolError`] when `k8s.get_secret` is registered already.
    pub fn register(
        registry: &ToolRegistry,
        clusters: Arc<dyn SecretClusters>,
    ) -> Result<(), RegisterToolError> {
        Self::register_with(registry, clusters, Arc::new(DenyReveal))
    }

    /// Registers the tool on `registry` with `policy`.
    ///
    /// # Errors
    ///
    /// [`RegisterToolError`] when `k8s.get_secret` is registered already.
    pub fn register_with(
        registry: &ToolRegistry,
        clusters: Arc<dyn SecretClusters>,
        policy: Arc<dyn RevealPolicy>,
    ) -> Result<(), RegisterToolError> {
        registry.register(Arc::new(Self::new(clusters, policy)))
    }
}

impl std::fmt::Debug for GetSecretTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetSecretTool").finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for GetSecretTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| OxiError::validation("`name` is required"))?
            .to_owned();
        let namespace = args
            .get("namespace")
            .and_then(Value::as_str)
            .filter(|ns| !ns.is_empty())
            .unwrap_or("default")
            .to_owned();
        let reveal = args.get("reveal").and_then(Value::as_bool).unwrap_or(false);
        if reveal && !self.policy.allows(ctx) {
            return Ok(ToolOutput::error(
                "Revealing Secret values is not permitted for this caller. Call without \
                 `reveal` to get the type, the key names and sizes.",
            ));
        }
        let (cluster, reader) = match self.clusters.reader(ctx.cluster.as_ref()) {
            Ok(found) => found,
            Err(error) if error.kind() == ErrorKind::Validation => return Err(error),
            Err(error) => return Ok(failure(&error)),
        };
        let target = ResourceRef::namespaced(
            cluster,
            Gvk::new("", "v1", "Secret"),
            namespace.as_str(),
            name.as_str(),
        );
        let resource = match reader
            .get(&target.gvk, target.namespace(), &target.name)
            .await
        {
            Ok(resource) => resource,
            Err(error) => return Ok(failure(&error)),
        };
        let summary = match self.service.summarize(&resource) {
            Ok(summary) => summary,
            Err(SecretError::NotASecret(found)) => {
                return Ok(ToolOutput::error(format!(
                    "`{name}` is a {found}, not a Secret"
                )));
            }
            Err(error) => return Ok(ToolOutput::error(error.to_string())),
        };
        let mut out = describe(&summary, &namespace, &name);
        if reveal {
            tracing::info!(
                namespace,
                name,
                "a Secret's values were returned to a tool caller"
            );
            let decoded = self
                .service
                .decode(&resource)
                .map_err(|error| OxiError::internal(error.to_string()))?;
            let values: Map<String, Value> = decoded
                .entries
                .iter()
                .map(|entry| (entry.key.clone(), Value::String(entry.value.shown())))
                .collect();
            out["values"] = Value::Object(values);
            out["masked"] = Value::Bool(false);
        }
        Ok(ToolOutput::json(out))
    }
}

/// A tool error the model sees: what failed.
fn failure(error: &OxiError) -> ToolOutput {
    ToolOutput::error(format!(
        "{}: {}",
        error.kind(),
        oxikube_domain::redact::redact(error.message())
    ))
}

/// The masked answer: type, keys with sizes and the type's public facts.
fn describe(summary: &SecretSummary, namespace: &str, name: &str) -> Value {
    let keys: Vec<Value> = summary
        .keys
        .iter()
        .map(|key| {
            json!({"key": key.key, "bytes": key.len, "binary": key.binary, "value": "(masked)"})
        })
        .collect();
    let mut out = json!({
        "name": name,
        "namespace": namespace,
        "type": summary.secret_type.as_str(),
        "immutable": summary.immutable,
        "masked": true,
        "keys": keys,
    });
    match &summary.details {
        TypeSummary::Registry(logins) => {
            out["registries"] = logins
                .iter()
                .map(|login| {
                    json!({
                        "server": login.server,
                        "username": login.username,
                        "has_credentials": login.has_credentials,
                    })
                })
                .collect();
        }
        TypeSummary::Tls(info) => {
            out["certificate"] = json!({
                "subject": info.subject,
                "issuer": info.issuer,
                "not_before": info.not_before.to_string(),
                "not_after": info.not_after.to_string(),
                "dns_names": info.dns_names,
            });
        }
        TypeSummary::BasicAuth { username } => out["username"] = json!(username),
        TypeSummary::ServiceAccountToken { service_account } => {
            out["service_account"] = json!(service_account);
        }
        _ => {}
    }
    out
}

/// The tool's definition: read-only, masked by default.
fn definition() -> ToolDef {
    let name = ToolName::new(GET_SECRET).expect("a well-formed tool name");
    ToolDef::read_only(
        name,
        "Read one Secret. By default the answer is masked: the type, each key's name and size, \
         and the type's public facts (registry servers and usernames, a TLS certificate's \
         subject and expiry), never a value. Pass `reveal: true` to ask for decoded text \
         values; that is refused unless the user has permitted this caller to read Secret \
         values. Binary values are never returned, only their size.",
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "The Secret's name."},
                "namespace": {
                    "type": "string",
                    "description": "The Secret's namespace. Default `default`."
                },
                "reveal": {
                    "type": "boolean",
                    "description": "Ask for the decoded values. Refused unless permitted. Default false."
                }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
    )
    .with_title("Get Secret")
    .with_annotations(ToolAnnotations {
        idempotent: true,
        ..ToolAnnotations::default()
    })
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::ContextName;
    use oxikube_domain::{Initiator, Resource};
    use oxikube_testkit::FakeResourcePort;

    use super::*;

    struct Fixed(Arc<FakeResourcePort>);

    impl SecretClusters for Fixed {
        fn reader(
            &self,
            cluster: Option<&ClusterId>,
        ) -> OxiResult<(ClusterId, Arc<dyn ResourceReader>)> {
            Ok((
                cluster
                    .cloned()
                    .unwrap_or_else(|| ClusterId::new("~/.kube/config", &ContextName::new("kind"))),
                self.0.clone(),
            ))
        }
    }

    struct Allow;

    impl RevealPolicy for Allow {
        fn allows(&self, ctx: &ToolContext) -> bool {
            ctx.initiator == Initiator::Agent
        }
    }

    fn secret() -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "db", "namespace": "ns"},
            "type": "Opaque",
            "data": {"password": "aHVudGVyMg==", "blob": "//4="}
        }))
        .unwrap()
    }

    fn registry(policy: Arc<dyn RevealPolicy>) -> ToolRegistry {
        let port = Arc::new(FakeResourcePort::new().with_objects([secret()]));
        let registry = ToolRegistry::new();
        GetSecretTool::register_with(&registry, Arc::new(Fixed(port)), policy).unwrap();
        registry
    }

    fn call(registry: &ToolRegistry, args: Value) -> OxiResult<ToolOutput> {
        futures::executor::block_on(registry.invoke(GET_SECRET, args, &ToolContext::agent()))
    }

    #[test]
    fn the_tool_is_a_registered_read_only_tool() {
        let registry = registry(Arc::new(DenyReveal));
        let def = registry
            .defs()
            .into_iter()
            .find(|d| d.name.as_str() == GET_SECRET)
            .expect("registered");
        assert!(!def.is_mutating() && def.read_only_hint());
        def.validate().unwrap();
        assert!(def.input_schema["properties"]["reveal"].is_object());
    }

    #[test]
    fn the_default_answer_is_masked() {
        let registry = registry(Arc::new(DenyReveal));
        let output = call(&registry, json!({"name": "db", "namespace": "ns"})).unwrap();
        assert!(!output.is_error);
        let text = output.content[0].as_text().unwrap();
        for leaked in ["hunter2", "aHVudGVyMg", "//4="] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
        let data = output.structured.unwrap();
        assert_eq!(data["masked"], true);
        assert_eq!(data["keys"][1]["key"], "password");
        assert_eq!(data["keys"][1]["bytes"], 7);
        assert_eq!(data["keys"][1]["value"], "(masked)");
        assert!(data.get("values").is_none());
    }

    #[test]
    fn asking_to_reveal_is_refused_by_default() {
        let registry = registry(Arc::new(DenyReveal));
        let output = call(
            &registry,
            json!({"name": "db", "namespace": "ns", "reveal": true}),
        )
        .unwrap();
        assert!(output.is_error);
        let text = output.content[0].as_text().unwrap();
        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("not permitted"), "{text}");
    }

    #[test]
    fn a_policy_that_allows_it_returns_text_values_and_sizes_of_binary_ones() {
        let registry = registry(Arc::new(Allow));
        let output = call(
            &registry,
            json!({"name": "db", "namespace": "ns", "reveal": true}),
        )
        .unwrap();
        assert!(!output.is_error);
        let data = output.structured.unwrap();
        assert_eq!(data["masked"], false);
        assert_eq!(data["values"]["password"], "hunter2");
        assert_eq!(data["values"]["blob"], "(binary, 2 bytes)");
    }

    #[test]
    fn a_missing_secret_is_a_tool_error_and_a_missing_name_a_failed_call() {
        let registry = registry(Arc::new(DenyReveal));
        let output = call(&registry, json!({"name": "nope", "namespace": "ns"})).unwrap();
        assert!(output.is_error);
        assert!(call(&registry, json!({})).is_err());
    }
}
