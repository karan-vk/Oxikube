//! Tests of the apply pipeline over the real guard and the testkit fakes: [`Applying`] is the
//! guard harness with the real `resource::Apply` handler and an [`ApplyService`] on it.

mod guard;
mod handler;
mod service;

use std::sync::Arc;

use futures::FutureExt as _;
use oxikube_domain::Resource;
use oxikube_testkit::{FakeResourcePort, ResourceCall};
use serde_json::{Value, json};

use super::{ApplyError, ApplyPreview, ApplyReport, ApplyRequest, ApplyService};
use crate::testing::{Harness, id};

/// The harness, the service, and the fakes of cluster `a`.
pub(super) struct Applying {
    pub h: Harness,
    pub service: ApplyService,
}

impl Applying {
    /// Cluster `a` connected (read-only when asked), discovery serving the core kinds.
    pub fn new(read_only: bool) -> Self {
        let h =
            Harness::with_extra(|reg, _, sessions| super::register_commands(reg, sessions.clone()));
        h.connect("a", read_only);
        h.connector
            .ports_for(&id("a"))
            .discovery
            .set_kinds(oxikube_testkit::kinds::core_kinds());
        let service = ApplyService::new(h.bus.clone(), h.manager.clone(), "alice");
        Self { h, service }
    }

    pub fn port(&self) -> Arc<FakeResourcePort> {
        self.h.resources("a")
    }

    pub fn request(&self, manifest: &str) -> ApplyRequest {
        ApplyRequest::new(id("a"), manifest)
    }

    pub fn preview(&self, request: ApplyRequest) -> Result<ApplyPreview, ApplyError> {
        self.service
            .preview(request)
            .now_or_never()
            .expect("the fakes answer at once")
    }

    pub fn apply(&self, preview: &ApplyPreview) -> Result<ApplyReport, ApplyError> {
        self.service
            .apply(preview)
            .now_or_never()
            .expect("the fakes answer at once")
    }

    /// Every call on the port, reads included.
    pub fn calls(&self) -> Vec<ResourceCall> {
        self.port().recorded_calls()
    }

    /// The writes, as `(method, dry_run)`.
    pub fn writes(&self) -> Vec<(&'static str, bool)> {
        self.h
            .writes("a")
            .iter()
            .map(|call| (method(call), call.is_dry_run()))
            .collect()
    }
}

fn method(call: &ResourceCall) -> &'static str {
    match call {
        ResourceCall::Patch { .. } => "patch",
        ResourceCall::Replace { .. } => "replace",
        ResourceCall::Create { .. } => "create",
        _ => "other",
    }
}

pub(super) const CONFIGMAP: &str = "\
# a comment the server never sees
apiVersion: v1
kind: ConfigMap
metadata:
  name: web
data:
  mode: blue
";

pub(super) fn live_configmap(version: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": "web",
            "namespace": "default",
            "resourceVersion": version,
            "managedFields": [
                {"manager": "kubectl", "operation": "Apply", "time": "2026-10-01T10:00:00Z"},
                {"manager": "kubectl", "operation": "Update", "time": "2026-10-02T11:00:00Z"},
                {"manager": "helm", "operation": "Update", "time": "2026-09-30T09:00:00Z"}
            ]
        },
        "data": {"mode": "green"}
    }))
    .expect("a valid object")
}

/// The body of a patch, replace or create call.
pub(super) fn body(call: &ResourceCall) -> &Value {
    match call {
        ResourceCall::Patch { patch, .. } => &patch.body,
        ResourceCall::Replace { object, .. } | ResourceCall::Create { object, .. } => object,
        other => panic!("not a write with a body: {other:?}"),
    }
}
