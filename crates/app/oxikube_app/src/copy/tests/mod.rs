//! Tests of the copy flow over the real guard, the real handlers and the testkit fakes: two
//! clusters, `a` the source and `b` the target, each with its own fake resource port.

mod audit;
mod conflict;
mod flow;
mod guard;
mod target;

use std::sync::Arc;

use futures::FutureExt as _;
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::{FakeResourcePort, ResourceCall};
use serde_json::{Value, json};

use super::{
    CopyChoice, CopyError, CopyOutcome, CopyPreview, CopyRequest, CopyService, ReviewAnswer,
    register_commands,
};
use crate::guard::ConfirmationProvider;
use crate::testing::{Harness, id};

/// The harness, the service, and the fakes of clusters `a` and `b`.
pub(super) struct Copying {
    pub h: Harness,
    pub service: CopyService,
}

impl Copying {
    /// `a` (the source) and `b` (the target, read-only when asked), both serving the core kinds;
    /// `b` has the namespaces `prod` and `staging`, `a` has `shop`.
    pub fn new(target_read_only: bool) -> Self {
        let h = Harness::with_extra(|registry, _, sessions| {
            register_commands(registry, sessions.clone())?;
            crate::namespaces::register_commands(registry)
        });
        h.connect("a", false);
        h.connect("b", target_read_only);
        for name in ["a", "b"] {
            h.connector
                .ports_for(&id(name))
                .discovery
                .set_kinds(oxikube_testkit::kinds::core_kinds());
        }
        let t = Self {
            service: CopyService::new(h.bus.clone(), h.manager.clone(), "alice"),
            h,
        };
        for namespace in ["prod", "staging"] {
            t.target().insert(namespace_object(namespace));
        }
        t.source().insert(namespace_object("shop"));
        t
    }

    pub fn source(&self) -> Arc<FakeResourcePort> {
        self.h.resources("a")
    }

    pub fn target(&self) -> Arc<FakeResourcePort> {
        self.h.resources("b")
    }

    /// A copy of `object` (inserted into the source) to namespace `namespace` of `b`.
    pub fn request(&self, object: &Value, namespace: &str) -> CopyRequest {
        self.source()
            .insert(Resource::from_json(object.clone()).expect("a valid object"));
        CopyRequest::to(source_ref(object), id("b"), Some(namespace))
    }

    pub fn preview(&self, request: CopyRequest) -> Result<CopyPreview, CopyError> {
        self.service
            .preview(request)
            .now_or_never()
            .expect("the fakes answer at once")
    }

    pub fn copy(
        &self,
        preview: &CopyPreview,
        choice: Option<CopyChoice>,
    ) -> Result<CopyOutcome, CopyError> {
        self.copy_with(preview, choice, ReviewAnswer::click())
    }

    pub fn copy_with(
        &self,
        preview: &CopyPreview,
        choice: Option<CopyChoice>,
        confirm: Arc<dyn ConfirmationProvider>,
    ) -> Result<CopyOutcome, CopyError> {
        self.service
            .copy(preview, choice, confirm)
            .now_or_never()
            .expect("the fakes answer at once")
    }

    /// The writes that reached the target for real (dry runs left out), as `(method, name)`.
    pub fn target_writes(&self) -> Vec<(&'static str, String)> {
        self.h
            .writes("b")
            .iter()
            .filter(|call| !call.is_dry_run())
            .map(|call| {
                (
                    method(call),
                    body(call)["metadata"]["name"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// Whether anything at all was sent to the source besides reads.
    pub fn source_writes(&self) -> usize {
        self.h.writes("a").len()
    }
}

pub(super) fn method(call: &ResourceCall) -> &'static str {
    match call {
        ResourceCall::Patch { .. } => "patch",
        ResourceCall::Replace { .. } => "replace",
        ResourceCall::Create { .. } => "create",
        _ => "other",
    }
}

/// The body of a patch, replace or create call.
pub(super) fn body(call: &ResourceCall) -> &Value {
    match call {
        ResourceCall::Patch { patch, .. } => &patch.body,
        ResourceCall::Replace { object, .. } | ResourceCall::Create { object, .. } => object,
        other => panic!("not a write with a body: {other:?}"),
    }
}

pub(super) fn namespace_object(name: &str) -> Resource {
    Resource::from_json(json!({"apiVersion": "v1", "kind": "Namespace",
        "metadata": {"name": name}}))
    .expect("a valid object")
}

/// The reference of `object` on cluster `a`.
pub(super) fn source_ref(object: &Value) -> ResourceRef {
    let text = |pointer: &str| object.pointer(pointer).and_then(Value::as_str);
    let gvk = Gvk::from_api_version(
        text("/apiVersion").unwrap_or_default(),
        text("/kind").unwrap_or_default(),
    );
    let name = text("/metadata/name").unwrap_or_default();
    match text("/metadata/namespace") {
        Some(namespace) => ResourceRef::namespaced(id("a"), gvk, namespace, name),
        None => ResourceRef::cluster_scoped(id("a"), gvk, name),
    }
}

/// A Deployment as a cluster returns it: everything the server owns is set.
pub(super) fn deployment() -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "web",
            "namespace": "shop",
            "uid": "7a1f",
            "resourceVersion": "4711",
            "generation": 3,
            "creationTimestamp": "2026-10-01T10:00:00Z",
            "selfLink": "/apis/apps/v1/namespaces/shop/deployments/web",
            "labels": {"app": "web"},
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "{\"old\":true}",
                "deployment.kubernetes.io/revision": "3",
                "team": "payments"
            },
            "ownerReferences": [{"apiVersion": "v1", "kind": "Namespace", "name": "shop", "uid": "x"}],
            "finalizers": ["example.com/hold"],
            "managedFields": [{"manager": "kubectl", "operation": "Update"}]
        },
        "spec": {
            "replicas": 2,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {
                "metadata": {"labels": {"app": "web"}},
                "spec": {"containers": [{"name": "web", "image": "nginx:1.27"}]}
            }
        },
        "status": {"readyReplicas": 2}
    })
}

pub(super) fn configmap() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "shop", "uid": "1", "resourceVersion": "9",
            "labels": {"tier": "backend"}},
        "data": {"mode": "blue"}
    })
}
