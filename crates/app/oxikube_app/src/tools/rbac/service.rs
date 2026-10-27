//! [`CanIService`]: ask a connected cluster a can-i question.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::access::AccessCheck;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::{AliasTarget, OxiError, OxiResult};
use oxikube_ports::ClockPort;

use super::answer::{CanIAnswer, CanIMethod};
use super::request::CanIRequest;
use super::subject::CanISubject;
use crate::search::aliases::AliasRegistry;
use crate::session::ClusterSessionManager;
use crate::tools::metrics::{session_for, within};

/// How long one review may take before the question is given up as timed out.
pub const CAN_I_DEADLINE: Duration = Duration::from_secs(15);

/// Answers can-i questions for the panel, the `k8s.can_i` tool and the `@rbac` mention. Cheap to
/// clone; clones share the sessions.
#[derive(Clone)]
pub struct CanIService {
    sessions: ClusterSessionManager,
    aliases: AliasRegistry,
    clock: Arc<dyn ClockPort>,
}

impl std::fmt::Debug for CanIService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanIService").finish_non_exhaustive()
    }
}

impl CanIService {
    /// A service over `sessions`, reading kind words from `aliases` and timed on `clock`.
    pub fn new(
        sessions: ClusterSessionManager,
        aliases: AliasRegistry,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            sessions,
            aliases,
            clock,
        }
    }

    /// The sessions questions are asked of.
    pub fn sessions(&self) -> &ClusterSessionManager {
        &self.sessions
    }

    /// The request that asks `verb` (`get` when `None`) on `target`: its resource spelled as
    /// `kubectl` does, its namespace and name. Nothing is asked of the cluster.
    pub fn prefill(
        &self,
        cluster: &ClusterId,
        target: &ResourceRef,
        verb: Option<&str>,
    ) -> CanIRequest {
        let table = self.aliases.table(cluster);
        let kind = target.gvk.kind.to_ascii_lowercase();
        let resource = match table.resolve(&kind).target() {
            Some(AliasTarget::Gvr(gvr)) if gvr.group == target.gvk.group => {
                let mut text = gvr.resource.to_string();
                if !gvr.group.is_empty() {
                    text.push('.');
                    text.push_str(&gvr.group);
                }
                text
            }
            _ => kind,
        };
        CanIRequest {
            verb: verb.unwrap_or("get").to_owned(),
            resource,
            namespace: target.namespace().map(str::to_owned),
            name: Some(target.name.to_string()),
            subject: CanISubject::Myself,
        }
    }

    /// Resolves `request` into the check it asks of `cluster`.
    ///
    /// # Errors
    ///
    /// A validation error for an empty verb or resource.
    pub fn check_of(&self, cluster: &ClusterId, request: &CanIRequest) -> OxiResult<AccessCheck> {
        request.to_check(&self.aliases.table(cluster))
    }

    /// Asks `cluster` whether `request` is allowed, with a `SelfSubjectAccessReview` for yourself
    /// or a `SubjectAccessReview` for another subject. The request is aborted when the returned
    /// future is dropped.
    ///
    /// # Errors
    ///
    /// `Validation` for a malformed request, `Network` for a cluster that is not connected,
    /// `Forbidden` when a check as another subject lacks permission to create a
    /// `SubjectAccessReview`, `Timeout` after [`CAN_I_DEADLINE`], and the review's own failures.
    pub async fn ask(&self, cluster: &ClusterId, request: &CanIRequest) -> OxiResult<CanIAnswer> {
        let check = self.check_of(cluster, request)?;
        let session = session_for(&self.sessions, Some(cluster))?;
        let port = session
            .access()
            .ok_or_else(|| OxiError::network("the cluster is not connected"))?;
        let method = if check.impersonate.is_some() {
            CanIMethod::SubjectAccessReview
        } else {
            CanIMethod::SelfSubjectAccessReview
        };
        let decision = within(self.clock.as_ref(), CAN_I_DEADLINE, port.can_i(&check))
            .await
            .ok_or_else(|| OxiError::timeout("the access review timed out"))??;
        Ok(CanIAnswer {
            check,
            subject: request.subject.clone(),
            method,
            decision,
        })
    }
}
