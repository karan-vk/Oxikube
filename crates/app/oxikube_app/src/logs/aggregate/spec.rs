//! [`AggregateSpec`]: what a multi-pod log reads, and which objects name a set of pods.

use std::fmt;

use oxikube_domain::ids::{Gvk, ResourceRef};

/// Whether `gvk` is an object whose selector names pods: Deployment, StatefulSet, DaemonSet,
/// ReplicaSet, Job, ReplicationController and Service.
pub fn is_aggregate_kind(gvk: &Gvk) -> bool {
    matches!(
        (&*gvk.group, &*gvk.kind),
        (
            "apps",
            "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet"
        ) | ("batch", "Job")
            | ("", "Service" | "ReplicationController")
    )
}

/// Where the pods of an aggregate come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateSource {
    /// A workload or Service of the namespace: its own selector picks the pods.
    Object {
        /// The object's type (one for which [`is_aggregate_kind`] holds).
        gvk: Gvk,
        /// The object's name.
        name: String,
    },
    /// A label selector as typed (`app=web,tier!=db`).
    Selector(String),
}

/// What a multi-pod session reads: the pods of one namespace picked by a selector, optionally
/// narrowed by a further label selector and by container name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateSpec {
    /// Namespace of the pods.
    pub namespace: String,
    /// Where the selector comes from.
    pub source: AggregateSource,
    /// A further label selector the pods must match too (AND).
    pub extra_selector: Option<String>,
    /// Read only the containers of this name (every container when `None`).
    pub container: Option<String>,
}

impl AggregateSpec {
    /// The pods of the workload or Service `object`. `None` for a cluster-scoped reference or a
    /// kind whose selector does not name pods.
    pub fn of(object: &ResourceRef) -> Option<Self> {
        if !is_aggregate_kind(&object.gvk) {
            return None;
        }
        Some(Self {
            namespace: object.namespace.as_deref()?.to_owned(),
            source: AggregateSource::Object {
                gvk: object.gvk.clone(),
                name: object.name.to_string(),
            },
            extra_selector: None,
            container: None,
        })
    }

    /// The pods of `namespace` that match the label selector `selector` as typed.
    pub fn selector(namespace: impl Into<String>, selector: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            source: AggregateSource::Selector(selector.into()),
            extra_selector: None,
            container: None,
        }
    }

    /// Narrows the pods by one more label selector.
    #[must_use]
    pub fn also_matching(mut self, selector: impl Into<String>) -> Self {
        self.extra_selector = Some(selector.into()).filter(|s: &String| !s.trim().is_empty());
        self
    }

    /// Reads only the containers named `container`.
    #[must_use]
    pub fn container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    /// The short name the session is listed by: `deployment/api`, `service/web`, or
    /// `selector app=web` (the way kubectl names them).
    pub fn label(&self) -> String {
        match &self.source {
            AggregateSource::Object { gvk, name } => {
                format!("{}/{name}", gvk.kind.to_ascii_lowercase())
            }
            AggregateSource::Selector(selector) => format!("selector {selector}"),
        }
    }
}

impl fmt::Display for AggregateSpec {
    /// `namespace/deployment/api`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.label())
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::{ClusterId, ContextName};

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new("~/.kube/config", &ContextName::new("kind"))
    }

    fn object(group: &str, kind: &str) -> ResourceRef {
        ResourceRef::namespaced(cluster(), Gvk::new(group, "v1", kind), "shop", "api")
    }

    #[test]
    fn workloads_and_services_name_pods_and_nothing_else_does() {
        for (group, kind) in [
            ("apps", "Deployment"),
            ("apps", "StatefulSet"),
            ("apps", "DaemonSet"),
            ("apps", "ReplicaSet"),
            ("batch", "Job"),
            ("", "Service"),
            ("", "ReplicationController"),
        ] {
            let spec = AggregateSpec::of(&object(group, kind)).expect(kind);
            assert_eq!(spec.namespace, "shop");
        }
        for (group, kind) in [("", "Pod"), ("", "ConfigMap"), ("batch", "CronJob")] {
            assert!(AggregateSpec::of(&object(group, kind)).is_none(), "{kind}");
        }
        let cluster_scoped =
            ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "n");
        assert!(AggregateSpec::of(&cluster_scoped).is_none());
    }

    #[test]
    fn the_label_and_display_say_what_is_read() {
        let spec = AggregateSpec::of(&object("apps", "Deployment")).unwrap();
        assert_eq!(spec.label(), "deployment/api");
        assert_eq!(spec.to_string(), "shop/deployment/api");
        let spec = AggregateSpec::selector("shop", "app=web");
        assert_eq!(spec.label(), "selector app=web");
        assert_eq!(
            AggregateSpec::selector("shop", "app=web")
                .also_matching("  ")
                .extra_selector,
            None,
            "a blank narrowing selector is none"
        );
    }
}
