//! `k8s.can_i`: the can-i question as a read-only tool.
//!
//! | Argument | Meaning |
//! |---|---|
//! | `verb` | `get`, `list`, `delete`, ... (required) |
//! | `resource` | as `kubectl`: `pods`, `po`, `pods/exec`, `deployments.apps` (required) |
//! | `namespace` | the namespace; omit for a cluster-scoped resource |
//! | `name` | one object's name |
//! | `subject` | `{kind: user\|group\|serviceaccount, name, namespace?}`; omit to ask as the current user |
//!
//! The structured result is `{allowed, decision, reason, method, question, kubectl}`: `decision`
//! is `allowed`, `denied` or `unknown` (an authorizer that could not decide is never reported as
//! denied), `reason` is the API server's own, and `method` says which review answered. A question
//! the cluster could not answer (not connected, no permission to create a `SubjectAccessReview`)
//! is a tool error the model sees.

use async_trait::async_trait;
use oxikube_domain::redact::redact;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{ToolAnnotations, ToolContext, ToolDef, ToolName, ToolOutput, ToolPort};
use serde_json::{Value, json};

use super::request::CanIRequest;
use super::service::CanIService;
use crate::tools::metrics::session_for;

/// The tool's name.
pub const CAN_I: &str = "k8s.can_i";

/// The `k8s.can_i` tool: see the [module docs](self).
pub struct CanITool {
    def: ToolDef,
    service: CanIService,
}

impl CanITool {
    /// The tool, asking through `service`.
    pub fn new(service: CanIService) -> Self {
        Self {
            def: definition(),
            service,
        }
    }

    /// The cluster a call is about: the context's, else the only connected one.
    fn cluster(
        &self,
        sessions: &crate::session::ClusterSessionManager,
        ctx: &ToolContext,
    ) -> OxiResult<oxikube_domain::ids::ClusterId> {
        Ok(session_for(sessions, ctx.cluster.as_ref())?.id().clone())
    }
}

impl std::fmt::Debug for CanITool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanITool").finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for CanITool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let request = CanIRequest::from_args(&args)?;
        let cluster = match self.cluster(self.service.sessions(), ctx) {
            Ok(cluster) => cluster,
            Err(error) if error.kind() == ErrorKind::Validation => return Err(error),
            Err(error) => return Ok(failure(&error)),
        };
        match self.service.ask(&cluster, &request).await {
            Ok(answer) => {
                let mut out = answer.to_json();
                out["cluster"] = json!(cluster);
                Ok(ToolOutput::json(out))
            }
            Err(error) if error.kind() == ErrorKind::Validation => Err(error),
            Err(error) => Ok(failure(&error)),
        }
    }
}

/// A tool error the model sees: what failed.
fn failure(error: &OxiError) -> ToolOutput {
    ToolOutput::error(format!("{}: {}", error.kind(), redact(error.message())))
}

fn definition() -> ToolDef {
    let name = ToolName::new(CAN_I).expect("a well-formed tool name");
    ToolDef::read_only(
        name,
        "Ask whether the current user, or another User, Group or ServiceAccount, may perform a \
         verb on a resource (as `kubectl auth can-i`). The answer is allowed, denied or unknown, \
         with the API server's reason and which access review answered. Read-only: it changes \
         nothing in the cluster. Asking as another subject needs permission to create a \
         SubjectAccessReview.",
        json!({
            "type": "object",
            "properties": {
                "verb": {"type": "string", "minLength": 1, "description": "get, list, watch, create, update, patch, delete, ... or *."},
                "resource": {"type": "string", "minLength": 1, "description": "As kubectl: pods, po, pods/exec, deployments.apps."},
                "namespace": {"type": "string", "description": "The namespace. Omit for a cluster-scoped resource."},
                "name": {"type": "string", "description": "One object's name. Omit for the kind as a whole."},
                "subject": {
                    "type": "object",
                    "description": "Ask as this subject. Omit to ask as the current user.",
                    "properties": {
                        "kind": {"type": "string", "enum": ["user", "group", "serviceaccount"]},
                        "name": {"type": "string", "minLength": 1},
                        "namespace": {"type": "string", "description": "A ServiceAccount's namespace."}
                    },
                    "required": ["kind", "name"],
                    "additionalProperties": false
                }
            },
            "required": ["verb", "resource"],
            "additionalProperties": false
        }),
    )
    .with_title("Can I?")
    .with_annotations(ToolAnnotations {
        idempotent: true,
        ..ToolAnnotations::default()
    })
}
