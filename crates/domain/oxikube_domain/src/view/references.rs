//! [`RefKind`] and [`pod_references`]: what a Pod points at by name (E12-S18), the input of the
//! reverse "used by" index.
//!
//! The five kinds are the ones a user asks "what uses this?" about before deleting it: the
//! ServiceAccount, ConfigMap, Secret and PersistentVolumeClaim of a Pod's namespace, and the
//! cluster-scoped PriorityClass. For each Pod, [`pod_references`] reads, in place:
//!
//! | Source | Reference |
//! |---|---|
//! | `spec.serviceAccountName` (else the deprecated `spec.serviceAccount`) | ServiceAccount |
//! | `spec.priorityClassName` | PriorityClass |
//! | `spec.imagePullSecrets[].name` | Secret |
//! | `spec.volumes[].persistentVolumeClaim.claimName`, and `<pod>-<volume>` for a generic ephemeral volume | PersistentVolumeClaim |
//! | `spec.volumes[].configMap.name`, `projected.sources[].configMap.name` | ConfigMap |
//! | `spec.volumes[].secret.secretName`, `projected.sources[].secret.name`, `csi.nodePublishSecretRef.name`, `flexVolume.secretRef.name` | Secret |
//! | `env[].valueFrom.configMapKeyRef.name`, `envFrom[].configMapRef.name` of every container, init and ephemeral ones included | ConfigMap |
//! | `env[].valueFrom.secretKeyRef.name`, `envFrom[].secretRef.name` likewise | Secret |
//!
//! An `optional` reference still counts: the Pod names the object, and deleting it changes what
//! the Pod gets. Nothing here reads a Secret's data; only the names the Pod spec holds.

use std::sync::Arc;

use super::storage::pod_claims;
use super::{arr_of, str_of, sub};
use crate::ids::Gvk;
use crate::json::JsonRef;
use crate::resource::Resource;

/// A kind a Pod can point at and whose "used by" the app can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefKind {
    /// A ServiceAccount (`spec.serviceAccountName`).
    ServiceAccount,
    /// A ConfigMap (a volume, a projected source, an `env` or `envFrom` entry).
    ConfigMap,
    /// A Secret (a volume, a projected source, an `env` or `envFrom` entry, an image pull secret).
    Secret,
    /// A PersistentVolumeClaim (a volume).
    PersistentVolumeClaim,
    /// A PriorityClass (`spec.priorityClassName`); cluster-scoped.
    PriorityClass,
}

impl RefKind {
    /// Every kind, in the order the index numbers them.
    pub const ALL: [RefKind; 5] = [
        RefKind::ServiceAccount,
        RefKind::ConfigMap,
        RefKind::Secret,
        RefKind::PersistentVolumeClaim,
        RefKind::PriorityClass,
    ];

    /// The kind `gvk` names, `None` for any other (a version is not compared).
    pub fn from_gvk(gvk: &Gvk) -> Option<Self> {
        match (&*gvk.group, &*gvk.kind) {
            ("", "ServiceAccount") => Some(RefKind::ServiceAccount),
            ("", "ConfigMap") => Some(RefKind::ConfigMap),
            ("", "Secret") => Some(RefKind::Secret),
            ("", "PersistentVolumeClaim") => Some(RefKind::PersistentVolumeClaim),
            ("scheduling.k8s.io", "PriorityClass") => Some(RefKind::PriorityClass),
            _ => None,
        }
    }

    /// The kind's type.
    pub fn gvk(self) -> Gvk {
        match self {
            RefKind::ServiceAccount => Gvk::new("", "v1", "ServiceAccount"),
            RefKind::ConfigMap => Gvk::new("", "v1", "ConfigMap"),
            RefKind::Secret => Gvk::new("", "v1", "Secret"),
            RefKind::PersistentVolumeClaim => Gvk::new("", "v1", "PersistentVolumeClaim"),
            RefKind::PriorityClass => Gvk::new("scheduling.k8s.io", "v1", "PriorityClass"),
        }
    }

    /// Whether objects of the kind live outside any namespace.
    pub const fn is_cluster_scoped(self) -> bool {
        matches!(self, RefKind::PriorityClass)
    }

    /// The position of the kind in [`ALL`](Self::ALL).
    pub const fn slot(self) -> usize {
        self as usize
    }

    /// The kind's name, as a user says it.
    pub const fn title(self) -> &'static str {
        match self {
            RefKind::ServiceAccount => "ServiceAccount",
            RefKind::ConfigMap => "ConfigMap",
            RefKind::Secret => "Secret",
            RefKind::PersistentVolumeClaim => "PersistentVolumeClaim",
            RefKind::PriorityClass => "PriorityClass",
        }
    }
}

/// One object a Pod points at: its kind and name (the namespace is the Pod's, except for a
/// [cluster-scoped](RefKind::is_cluster_scoped) kind).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PodRef {
    /// What kind of object.
    pub kind: RefKind,
    /// Its name.
    pub name: Arc<str>,
}

/// Everything `pod` names among the five [`RefKind`]s, sorted by kind then name, without repeats.
/// Empty for anything that is not a Pod spec with references.
///
/// Reads the JSON in place and allocates only the names, so it is cheap enough to run once per
/// Pod delta; whether the Pod is still active is [`pod_is_active`](super::storage::pod_is_active)'s.
pub fn pod_references(pod: &Resource) -> Vec<PodRef> {
    let mut out: Vec<PodRef> = Vec::with_capacity(8);
    let spec = sub(pod.json(), "spec");
    if let Some(account) =
        str_of(spec, "serviceAccountName").or_else(|| str_of(spec, "serviceAccount"))
    {
        push(&mut out, RefKind::ServiceAccount, account);
    }
    if let Some(class) = str_of(spec, "priorityClassName") {
        push(&mut out, RefKind::PriorityClass, class);
    }
    for secret in arr_of(spec, "imagePullSecrets").iter() {
        if let Some(name) = str_of(secret, "name") {
            push(&mut out, RefKind::Secret, name);
        }
    }
    for volume in arr_of(spec, "volumes").iter() {
        volume_refs(volume, &mut out);
    }
    for containers in ["containers", "initContainers", "ephemeralContainers"] {
        for container in arr_of(spec, containers).iter() {
            container_refs(container, &mut out);
        }
    }
    for claim in pod_claims(pod) {
        out.push(PodRef {
            kind: RefKind::PersistentVolumeClaim,
            name: claim,
        });
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn push(out: &mut Vec<PodRef>, kind: RefKind, name: &str) {
    out.push(PodRef {
        kind,
        name: Arc::from(name),
    });
}

/// The ConfigMaps and Secrets one volume names (its claims are [`pod_claims`]').
fn volume_refs(volume: JsonRef<'_>, out: &mut Vec<PodRef>) {
    if let Some(name) = str_of(sub(volume, "configMap"), "name") {
        push(out, RefKind::ConfigMap, name);
    }
    if let Some(name) = str_of(sub(volume, "secret"), "secretName") {
        push(out, RefKind::Secret, name);
    }
    if let Some(name) = str_of(sub(sub(volume, "csi"), "nodePublishSecretRef"), "name") {
        push(out, RefKind::Secret, name);
    }
    if let Some(name) = str_of(sub(sub(volume, "flexVolume"), "secretRef"), "name") {
        push(out, RefKind::Secret, name);
    }
    for source in arr_of(sub(volume, "projected"), "sources").iter() {
        if let Some(name) = str_of(sub(source, "configMap"), "name") {
            push(out, RefKind::ConfigMap, name);
        }
        if let Some(name) = str_of(sub(source, "secret"), "name") {
            push(out, RefKind::Secret, name);
        }
    }
}

/// The ConfigMaps and Secrets one container's `env` and `envFrom` name.
fn container_refs(container: JsonRef<'_>, out: &mut Vec<PodRef>) {
    for var in arr_of(container, "env").iter() {
        let from = sub(var, "valueFrom");
        if let Some(name) = str_of(sub(from, "configMapKeyRef"), "name") {
            push(out, RefKind::ConfigMap, name);
        }
        if let Some(name) = str_of(sub(from, "secretKeyRef"), "name") {
            push(out, RefKind::Secret, name);
        }
    }
    for source in arr_of(container, "envFrom").iter() {
        if let Some(name) = str_of(sub(source, "configMapRef"), "name") {
            push(out, RefKind::ConfigMap, name);
        }
        if let Some(name) = str_of(sub(source, "secretRef"), "name") {
            push(out, RefKind::Secret, name);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pod(spec: serde_json::Value) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "web-0", "namespace": "shop"},
            "spec": spec,
        }))
        .expect("a pod")
    }

    fn refs(pod: &Resource) -> Vec<(RefKind, String)> {
        pod_references(pod)
            .into_iter()
            .map(|r| (r.kind, r.name.to_string()))
            .collect()
    }

    #[test]
    fn the_service_account_and_priority_class_are_read() {
        let p = pod(json!({"serviceAccountName": "web", "priorityClassName": "high"}));
        assert_eq!(
            refs(&p),
            [
                (RefKind::ServiceAccount, "web".into()),
                (RefKind::PriorityClass, "high".into())
            ]
        );
        let old = pod(json!({"serviceAccount": "legacy"}));
        assert_eq!(refs(&old), [(RefKind::ServiceAccount, "legacy".into())]);
    }

    #[test]
    fn every_volume_source_is_read() {
        let p = pod(json!({"volumes": [
            {"name": "a", "persistentVolumeClaim": {"claimName": "data"}},
            {"name": "b", "configMap": {"name": "cfg"}},
            {"name": "c", "secret": {"secretName": "tls"}},
            {"name": "d", "ephemeral": {"volumeClaimTemplate": {}}},
            {"name": "e", "csi": {"driver": "x", "nodePublishSecretRef": {"name": "csi-creds"}}},
            {"name": "f", "flexVolume": {"driver": "y", "secretRef": {"name": "flex-creds"}}},
            {"name": "g", "emptyDir": {}},
        ]}));
        assert_eq!(
            refs(&p),
            [
                (RefKind::ConfigMap, "cfg".into()),
                (RefKind::Secret, "csi-creds".into()),
                (RefKind::Secret, "flex-creds".into()),
                (RefKind::Secret, "tls".into()),
                (RefKind::PersistentVolumeClaim, "data".into()),
                (RefKind::PersistentVolumeClaim, "web-0-d".into()),
            ]
        );
    }

    #[test]
    fn projected_sources_are_read() {
        let p = pod(json!({"volumes": [{"name": "p", "projected": {"sources": [
            {"configMap": {"name": "kube-root-ca.crt"}},
            {"secret": {"name": "token"}},
            {"serviceAccountToken": {"path": "t"}},
            {"downwardAPI": {"items": []}},
        ]}}]}));
        assert_eq!(
            refs(&p),
            [
                (RefKind::ConfigMap, "kube-root-ca.crt".into()),
                (RefKind::Secret, "token".into()),
            ]
        );
    }

    #[test]
    fn env_and_env_from_of_every_container_kind_are_read() {
        let container = |cm: &str, secret: &str, from_cm: &str, from_secret: &str| {
            json!({
                "name": "c",
                "env": [
                    {"name": "A", "value": "literal"},
                    {"name": "B", "valueFrom": {"configMapKeyRef": {"name": cm, "key": "k"}}},
                    {"name": "C", "valueFrom": {"secretKeyRef": {"name": secret, "key": "k", "optional": true}}},
                    {"name": "D", "valueFrom": {"fieldRef": {"fieldPath": "metadata.name"}}},
                ],
                "envFrom": [{"configMapRef": {"name": from_cm}}, {"secretRef": {"name": from_secret}}],
            })
        };
        let p = pod(json!({
            "containers": [container("cm1", "s1", "cm2", "s2")],
            "initContainers": [container("cm3", "s3", "cm4", "s4")],
            "ephemeralContainers": [container("cm5", "s5", "cm6", "s6")],
        }));
        let got = refs(&p);
        for n in 1..=6 {
            assert!(
                got.contains(&(RefKind::ConfigMap, format!("cm{n}"))),
                "cm{n}"
            );
            assert!(got.contains(&(RefKind::Secret, format!("s{n}"))), "s{n}");
        }
        assert_eq!(got.len(), 12);
    }

    #[test]
    fn image_pull_secrets_are_secrets_and_repeats_collapse() {
        let p = pod(json!({
            "imagePullSecrets": [{"name": "regcred"}, {"name": "regcred"}],
            "volumes": [{"name": "x", "secret": {"secretName": "regcred"}}],
        }));
        assert_eq!(refs(&p), [(RefKind::Secret, "regcred".into())]);
    }

    #[test]
    fn a_pod_without_references_has_none() {
        assert!(refs(&pod(json!({"containers": [{"name": "c"}]}))).is_empty());
        assert!(refs(&pod(json!({}))).is_empty());
    }

    #[test]
    fn kinds_round_trip_through_their_type() {
        for kind in RefKind::ALL {
            assert_eq!(RefKind::from_gvk(&kind.gvk()), Some(kind));
            assert_eq!(
                kind.slot(),
                RefKind::ALL.iter().position(|k| *k == kind).unwrap()
            );
        }
        assert_eq!(
            RefKind::from_gvk(&Gvk::new("apps", "v1", "Deployment")),
            None
        );
        assert_eq!(
            RefKind::from_gvk(&Gvk::new("example.com", "v1", "Secret")),
            None
        );
        assert!(RefKind::PriorityClass.is_cluster_scoped());
        assert!(!RefKind::Secret.is_cluster_scoped());
    }
}
