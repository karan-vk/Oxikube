//! Storage builders (E12-S15): claims, volumes, classes and Pods that mount claims, for the
//! storage view-model, column, command and UI tests.
//!
//! `pvc("data").namespace("shop").class("fast").requests("10Gi").capacity("10Gi")` builds a
//! bound claim; `pod_mounting("web-0", "shop", &["data"])` a running Pod with one volume per
//! claim. Every builder ends with `build()`.

use oxikube_domain::Resource;
use serde_json::{Value, json};

use crate::builders::{NAMESPACE, pod, resource};

/// Builder for a PersistentVolumeClaim, bound by default (class `standard`, 10Gi asked and
/// given, `ReadWriteOnce`).
#[derive(Debug, Clone)]
pub struct PvcBuilder {
    name: String,
    namespace: String,
    class: Option<String>,
    requested: Option<String>,
    capacity: Option<String>,
    phase: &'static str,
    volume: Option<String>,
    conditions: Vec<Value>,
}

/// A bound 10Gi claim named `name` in [`NAMESPACE`].
pub fn pvc(name: impl Into<String>) -> PvcBuilder {
    PvcBuilder {
        name: name.into(),
        namespace: NAMESPACE.to_owned(),
        class: Some("standard".to_owned()),
        requested: Some("10Gi".to_owned()),
        capacity: Some("10Gi".to_owned()),
        phase: "Bound",
        volume: Some("pvc-0f3b6e0e".to_owned()),
        conditions: Vec::new(),
    }
}

impl PvcBuilder {
    /// Sets the namespace.
    #[must_use]
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    /// Sets `spec.storageClassName`.
    #[must_use]
    pub fn class(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }

    /// Removes `spec.storageClassName`.
    #[must_use]
    pub fn no_class(mut self) -> Self {
        self.class = None;
        self
    }

    /// Sets the requested size.
    #[must_use]
    pub fn requests(mut self, size: impl Into<String>) -> Self {
        self.requested = Some(size.into());
        self
    }

    /// Sets the bound capacity.
    #[must_use]
    pub fn capacity(mut self, size: impl Into<String>) -> Self {
        self.capacity = Some(size.into());
        self
    }

    /// Makes the claim `Pending`: no volume, no capacity.
    #[must_use]
    pub fn pending(mut self) -> Self {
        self.phase = "Pending";
        self.volume = None;
        self.capacity = None;
        self
    }

    /// Adds a `status.conditions` entry.
    #[must_use]
    pub fn condition(mut self, kind: &str, status: &str, message: &str) -> Self {
        self.conditions
            .push(json!({"type": kind, "status": status, "message": message}));
        self
    }

    /// The object as JSON.
    pub fn json(&self) -> Value {
        let mut spec = json!({
            "accessModes": ["ReadWriteOnce"],
            "resources": {"requests": {}},
            "volumeMode": "Filesystem",
        });
        if let Some(requested) = &self.requested {
            spec["resources"]["requests"]["storage"] = json!(requested);
        }
        if let Some(class) = &self.class {
            spec["storageClassName"] = json!(class);
        }
        if let Some(volume) = &self.volume {
            spec["volumeName"] = json!(volume);
        }
        let mut status = json!({"phase": self.phase});
        if let Some(capacity) = &self.capacity {
            status["capacity"] = json!({"storage": capacity});
            status["accessModes"] = json!(["ReadWriteOnce"]);
        }
        if !self.conditions.is_empty() {
            status["conditions"] = json!(self.conditions);
        }
        resource("v1", "PersistentVolumeClaim")
            .name(&self.name)
            .namespace(&self.namespace)
            .field("spec", spec)
            .field("status", status)
            .json()
    }

    /// Builds the [`Resource`].
    ///
    /// # Panics
    ///
    /// Only if the builder produced JSON that is not a valid claim (a testkit bug).
    pub fn build(&self) -> Resource {
        Resource::from_json(self.json()).expect("a valid claim")
    }
}

/// Builder for a StorageClass (`ebs.csi.aws.com`, `Delete`, `Immediate`, no expansion).
#[derive(Debug, Clone)]
pub struct StorageClassBuilder {
    name: String,
    expansion: Option<bool>,
    default: Option<&'static str>,
}

/// A StorageClass named `name`.
pub fn storage_class(name: impl Into<String>) -> StorageClassBuilder {
    StorageClassBuilder {
        name: name.into(),
        expansion: None,
        default: None,
    }
}

impl StorageClassBuilder {
    /// Sets `allowVolumeExpansion`.
    #[must_use]
    pub fn expandable(mut self, allow: bool) -> Self {
        self.expansion = Some(allow);
        self
    }

    /// Marks it the default class with the stable annotation.
    #[must_use]
    pub fn default_class(mut self) -> Self {
        self.default = Some("storageclass.kubernetes.io/is-default-class");
        self
    }

    /// Marks it the default class with the legacy beta annotation.
    #[must_use]
    pub fn beta_default_class(mut self) -> Self {
        self.default = Some("storageclass.beta.kubernetes.io/is-default-class");
        self
    }

    /// Builds the [`Resource`].
    pub fn build(&self) -> Resource {
        let mut builder = resource("storage.k8s.io/v1", "StorageClass")
            .cluster_scoped()
            .name(&self.name)
            .field("provisioner", json!("ebs.csi.aws.com"))
            .field("reclaimPolicy", json!("Delete"))
            .field("volumeBindingMode", json!("Immediate"));
        if let Some(allow) = self.expansion {
            builder = builder.field("allowVolumeExpansion", json!(allow));
        }
        if let Some(key) = self.default {
            builder = builder.annotation(key, "true");
        }
        builder.build()
    }
}

/// A running Pod named `name` in `namespace` with one volume per claim in `claims` (and a
/// ConfigMap volume, which is not a claim).
pub fn pod_mounting(name: &str, namespace: &str, claims: &[&str]) -> Resource {
    pod_mounting_in(name, namespace, claims, "Running")
}

/// [`pod_mounting`] with a given `status.phase` (`Succeeded` for a finished Pod).
///
/// # Panics
///
/// Only if the built JSON is not a valid Pod (a testkit bug).
pub fn pod_mounting_in(name: &str, namespace: &str, claims: &[&str], phase: &str) -> Resource {
    let mut json = pod().name(name).namespace(namespace).json();
    let mut volumes: Vec<Value> = claims
        .iter()
        .enumerate()
        .map(|(i, claim)| json!({"name": format!("vol{i}"), "persistentVolumeClaim": {"claimName": claim}}))
        .collect();
    volumes.push(json!({"name": "cfg", "configMap": {"name": "cfg"}}));
    json["spec"]["volumes"] = json!(volumes);
    json["status"]["phase"] = json!(phase);
    Resource::from_json(json).expect("a valid pod")
}
