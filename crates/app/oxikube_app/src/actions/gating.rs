//! [`ActionAccess`]: the soft RBAC gate on mutating row actions (E17-S03).
//!
//! A mutating action is greyed when the cached rules review says the user may not run the verb
//! it needs on the kind ("no delete permission"). The gate reads [`AccessService::peek_stale`]:
//! no request, no waiting, and anything but a definite deny leaves the action enabled. It is a
//! convenience only: the `MutationGuard` and the API server still decide every mutation.

use oxikube_domain::access::{AccessCheck, AccessDecision};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvr;
use oxikube_domain::kinds::ResourceKind;

use crate::access::AccessService;

/// The verb (and subresource) a mutating command needs on the kind it acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequiredAccess {
    /// The RBAC verb: `delete`, `patch`, `update`, ...
    pub verb: &'static str,
    /// The subresource the verb is on (`scale`), if any.
    pub subresource: Option<&'static str>,
}

impl RequiredAccess {
    const fn verb(verb: &'static str) -> Self {
        Self {
            verb,
            subresource: None,
        }
    }
}

/// What `command` needs from RBAC on its kind, for the row actions that mutate one object.
/// `None` for a command without a single verb on the kind itself (it creates another kind, or
/// is not a row mutation): it is never greyed by RBAC here.
pub fn required_access(command: CommandId) -> Option<RequiredAccess> {
    match command {
        CommandId::RESOURCE_DELETE
        | CommandId::POD_DELETE
        | CommandId::POD_FORCE_DELETE
        | CommandId::NAMESPACE_DELETE
        | CommandId::SELECTION_DELETE => Some(RequiredAccess::verb("delete")),
        CommandId::WORKLOAD_SCALE => Some(RequiredAccess {
            verb: "update",
            subresource: Some("scale"),
        }),
        CommandId::POD_EVICT => Some(RequiredAccess {
            verb: "create",
            subresource: Some("eviction"),
        }),
        CommandId::WORKLOAD_RESTART
        | CommandId::WORKLOAD_SET_IMAGE
        | CommandId::WORKLOAD_ROLLBACK
        | CommandId::JOB_SUSPEND
        | CommandId::JOB_RESUME
        | CommandId::CRONJOB_SUSPEND
        | CommandId::CRONJOB_RESUME
        | CommandId::NODE_CORDON
        | CommandId::NODE_UNCORDON
        | CommandId::NODE_DRAIN
        | CommandId::PVC_RESIZE
        | CommandId::STORAGECLASS_SET_DEFAULT
        | CommandId::INGRESS_CLASS_SET_DEFAULT
        | CommandId::SELECTION_RESTART
        | CommandId::SELECTION_LABEL => Some(RequiredAccess::verb("patch")),
        _ => None,
    }
}

/// The cached access decisions for a selection of objects of one kind. Cheap to build; reads
/// the cache only.
#[derive(Clone, Debug, Default)]
pub struct ActionAccess {
    service: Option<AccessService>,
    /// The namespaces of the selected objects (`None`: a cluster-scoped object, or the
    /// namespace is not known). Empty is the same as one `None`.
    namespaces: Vec<Option<String>>,
}

impl ActionAccess {
    /// No gate: every action stays as the session says.
    pub fn none() -> Self {
        Self::default()
    }

    /// A gate over `service` for objects in `namespaces`.
    pub fn new(service: AccessService, namespaces: Vec<Option<String>>) -> Self {
        Self {
            service: Some(service),
            namespaces,
        }
    }

    /// Whether the cached reviews say the user may not do `need` on `kind` in any of the
    /// selection's namespaces. An unreviewed namespace, a partial review or a grant on named
    /// objects counts as allowed: only a clear deny greys an action.
    pub fn denies(&self, kind: &ResourceKind, need: RequiredAccess) -> bool {
        let Some(service) = &self.service else {
            return false;
        };
        let gvr = Gvr::new(&*kind.gvk.group, &*kind.gvk.version, kind.plural.as_str());
        let check = |namespace: Option<&str>| {
            let mut check = AccessCheck::new(need.verb, gvr.clone());
            if let Some(subresource) = need.subresource {
                check = check.subresource(subresource);
            }
            if let Some(namespace) = namespace.filter(|_| kind.namespaced) {
                check = check.namespace(namespace);
            }
            matches!(
                service.peek_stale(&check),
                Some(AccessDecision::Denied { .. })
            )
        };
        if self.namespaces.is_empty() {
            return check(None);
        }
        // The selection spans namespaces: grey only when none of them allows it.
        self.namespaces.iter().all(|ns| check(ns.as_deref()))
    }
}
