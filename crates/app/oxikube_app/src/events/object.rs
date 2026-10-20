//! [`ObjectIdentity`] and [`ObjectTarget`]: the object an event is about, and the object a
//! query is about.
//!
//! An event names its object twice: by `uid` (`involvedObject.uid`, `regarding.uid`) and by
//! kind, namespace and name (`involvedObject.kind`, `.namespace`, `.name`). Most writers give
//! both, but some leave the uid out, and then the event belongs to no uid index: it would show
//! in the events view and in no object's Events tab, and it would badge no row.
//!
//! [`ObjectIdentity`] is that name — kind, namespace and name — and the events index keeps the
//! events that named no uid under it, so an object's Events tab and its warning badge find
//! them too (E14 follow-up #704).
//!
//! An event whose uid is *another* object's is never about this one, even when it names it: an
//! object deleted and re-created under the same name is a different object, and the old
//! object's events stay with it. So an event is about a target when
//!
//! - it names the target's uid, or
//! - it gives no uid at all and names the target (kind, namespace and name).
//!
//! An event with a uid and a target without one are matched by name: the target's uid is
//! unknown, so the name is all there is.

use std::sync::Arc;

use oxikube_domain::event::Event;
use oxikube_domain::ids::ResourceRef;

/// The name of an object: its kind, its namespace (`None` for a cluster-scoped object) and its
/// name.
///
/// The kind is held lowercased and compared case-insensitively, like the events filter's kind
/// rule; the namespace and the name are compared exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectIdentity {
    kind: Arc<str>,
    namespace: Option<Arc<str>>,
    name: Arc<str>,
}

impl ObjectIdentity {
    /// The identity of the object `kind`, `namespace` and `name` name.
    pub fn new(kind: &str, namespace: Option<&str>, name: &str) -> Self {
        Self {
            kind: Arc::from(kind.to_ascii_lowercase()),
            namespace: namespace.map(Arc::from),
            name: Arc::from(name),
        }
    }

    /// The identity of the object `object` names.
    pub fn of(object: &ResourceRef) -> Self {
        Self::new(&object.gvk.kind, object.namespace(), &object.name)
    }

    /// The identity of the object `event` is about.
    pub fn of_event(event: &Event) -> Self {
        Self::new(
            &event.regarding.gvk.kind,
            event.regarding.namespace(),
            &event.regarding.name,
        )
    }

    /// The kind, lowercased.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The namespace; `None` for a cluster-scoped object.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// The name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether `event` is about the object this identity names.
    pub fn matches(&self, event: &Event) -> bool {
        event.regarding.gvk.kind.eq_ignore_ascii_case(&self.kind)
            && *event.regarding.name == *self.name
            && self.namespace.as_deref() == event.regarding.namespace()
    }
}

/// The object a query is about: its uid when it has one, and the name the events that gave no
/// uid are matched on.
///
/// A target with no uid matches the events that gave no uid and name it; a target with a uid
/// matches those and the events that carry its uid.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObjectTarget {
    uid: Option<Arc<str>>,
    identity: Option<ObjectIdentity>,
}

impl ObjectTarget {
    /// The object with this uid, and no name: only the events that carry the uid are about it.
    pub fn uid(uid: impl Into<Arc<str>>) -> Self {
        Self {
            uid: Some(uid.into()),
            identity: None,
        }
    }

    /// The object `identity` names, with the uid `uid` when it has one.
    pub fn named(uid: Option<Arc<str>>, identity: ObjectIdentity) -> Self {
        Self {
            uid,
            identity: Some(identity),
        }
    }

    /// The object `object` names, with the uid `uid` when it has one.
    pub fn of(object: &ResourceRef, uid: Option<Arc<str>>) -> Self {
        Self::named(uid, ObjectIdentity::of(object))
    }

    /// The uid of the object, when it has one.
    pub fn uid_value(&self) -> Option<&str> {
        self.uid.as_deref()
    }

    /// The name of the object, when it is known.
    pub fn identity(&self) -> Option<&ObjectIdentity> {
        self.identity.as_ref()
    }

    /// Whether `event` is about this object: it carries the target's uid, or it gives no uid
    /// and names the target.
    pub fn matches(&self, event: &Event) -> bool {
        match (event.regarding_uid.as_deref(), self.uid.as_deref()) {
            (Some(event_uid), Some(uid)) => event_uid == uid,
            _ => self
                .identity
                .as_ref()
                .is_some_and(|identity| identity.matches(event)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::ids::{ClusterId, ContextName, Gvk};

    fn cluster() -> ClusterId {
        ClusterId::new("/home/me/.kube/config", &ContextName::new("a"))
    }

    fn reference(namespace: Option<&str>, name: &str) -> ResourceRef {
        ResourceRef::new(
            cluster(),
            Gvk::new("", "v1", "Pod"),
            namespace.map(Arc::from),
            name,
        )
    }

    #[test]
    fn an_identity_is_the_kind_the_namespace_and_the_name() {
        let identity = ObjectIdentity::new("Pod", Some("shop"), "web-0");
        assert_eq!(identity.kind(), "pod");
        assert_eq!(identity.namespace(), Some("shop"));
        assert_eq!(identity.name(), "web-0");
        assert_eq!(
            identity,
            ObjectIdentity::of(&reference(Some("shop"), "web-0"))
        );
    }

    #[test]
    fn the_kind_is_compared_case_insensitively() {
        let identity = ObjectIdentity::new("Pod", Some("shop"), "web-0");
        let mut object = reference(Some("shop"), "web-0");
        object.gvk.kind = Arc::from("pod");
        assert_eq!(identity, ObjectIdentity::of(&object));
    }

    #[test]
    fn a_cluster_scoped_object_has_no_namespace() {
        let identity = ObjectIdentity::new("Node", None, "worker-1");
        assert_eq!(identity.namespace(), None);
        assert_eq!(
            identity,
            ObjectIdentity::of(&ResourceRef::new(
                cluster(),
                Gvk::new("", "v1", "Node"),
                None,
                "worker-1"
            ))
        );
    }
}
