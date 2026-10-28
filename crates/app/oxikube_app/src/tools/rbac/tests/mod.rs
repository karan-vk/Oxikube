//! Tests of `k8s.can_i`, `@rbac` and the can-i service over fake sessions.

mod provider;
mod request;
mod tool;

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId, ToolContext, ToolOutput};
use oxikube_testkit::{
    FakeAccessReviewPort, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort,
    FakeResourcePort,
};
use serde_json::{Value, json};

use super::{CanIDeps, CanIService, install};
use crate::context::ContextRegistry;
use crate::search::aliases::AliasRegistry;
use crate::session::ClusterSessionManager;
use crate::tools::ToolRegistry;

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("dev"))
}

/// A fake cluster `dev` with the can-i tool and `@rbac` installed.
pub(super) struct Harness {
    pub(super) sessions: ClusterSessionManager,
    pub(super) connector: Arc<FakeClusterConnectorPort>,
    pub(super) tools: ToolRegistry,
    pub(super) contexts: ContextRegistry,
    pub(super) service: CanIService,
}

impl Harness {
    pub(super) async fn connected() -> Self {
        let h = Self::new();
        h.sessions.connect(&cluster()).await.expect("connect");
        h
    }

    pub(super) fn new() -> Self {
        let entry = ClusterContext::new(
            cluster(),
            ContextName::new("dev"),
            SourceId("kubeconfig".into()),
        );
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let clock = Arc::new(FakeClockPort::default());
        let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
        let service = CanIService::new(sessions.clone(), AliasRegistry::new(), clock);
        let deps = CanIDeps {
            sessions: sessions.clone(),
            service: service.clone(),
        };
        let tools = ToolRegistry::new();
        let contexts = ContextRegistry::new();
        install(&tools, &contexts, &deps).expect("install");
        Self {
            sessions,
            connector,
            tools,
            contexts,
            service,
        }
    }

    pub(super) fn access(&self) -> Arc<FakeAccessReviewPort> {
        self.connector.ports_for(&cluster()).access
    }

    pub(super) fn objects(&self) -> Arc<FakeResourcePort> {
        self.connector.ports_for(&cluster()).resources
    }

    pub(super) async fn can_i(&self, args: Value) -> oxikube_domain::OxiResult<ToolOutput> {
        self.tools
            .invoke("k8s.can_i", args, &ToolContext::agent())
            .await
    }
}

pub(super) fn structured(out: &ToolOutput) -> &Value {
    out.structured.as_ref().expect("a structured result")
}

pub(super) fn text(out: &ToolOutput) -> String {
    out.content
        .iter()
        .filter_map(|part| match part {
            oxikube_ports::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn object(value: Value) -> Resource {
    Resource::from_json(value).expect("an object")
}

pub(super) fn role_binding(ns: &str, name: &str, role: &str, subjects: Value) -> Resource {
    object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
        "metadata": {"name": name, "namespace": ns},
        "roleRef": {"kind": "Role", "name": role, "apiGroup": "rbac.authorization.k8s.io"},
        "subjects": subjects,
    }))
}
