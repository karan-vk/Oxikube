//! [`AccessService`]: cached rules, `peek`, `can_i` and the batch `can_i_many`.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use oxikube_domain::OxiResult;
use oxikube_domain::access::{AccessCheck, AccessDecision, AccessRules};
use oxikube_ports::{AccessReviewPort, ClockPort};

use super::cache::RulesCache;

/// How long a rules review stays fresh: the same minute the adapter's own cache uses.
pub const DEFAULT_TTL: Duration = Duration::from_secs(60);

/// The most reviews in flight at once from one batch.
pub const MAX_IN_FLIGHT: usize = 4;

/// What the current user may do on one connected cluster. See the [module docs](super).
///
/// Cheap to clone; clones share the cache.
#[derive(Clone)]
pub struct AccessService {
    cache: Arc<RulesCache>,
}

impl std::fmt::Debug for AccessService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessService")
            .field("cached_namespaces", &self.cache.len())
            .finish_non_exhaustive()
    }
}

impl AccessService {
    /// A service over `port` whose reviews stay fresh for `ttl` on `clock`.
    pub fn new(port: Arc<dyn AccessReviewPort>, clock: Arc<dyn ClockPort>, ttl: Duration) -> Self {
        Self {
            cache: Arc::new(RulesCache::new(port, clock, ttl)),
        }
    }

    /// The rules of `namespace` (`None`: the cluster-wide ask, marked
    /// [`namespaces_unseen`](AccessRules::namespaces_unseen)). Cached for the TTL; concurrent
    /// asks share one review.
    ///
    /// # Errors
    ///
    /// The port's error (`Auth`, `Network`, ...). Nothing is cached for a failure.
    pub async fn rules(&self, namespace: Option<&str>) -> OxiResult<Arc<AccessRules>> {
        self.cache.get(namespace).await
    }

    /// The rules of `namespace` if a fresh review is cached; never waits or asks.
    pub fn peek_rules(&self, namespace: Option<&str>) -> Option<Arc<AccessRules>> {
        self.cache.peek(namespace)
    }

    /// Answers `check` from the cached rules alone: `None` when the namespace was not reviewed
    /// (or the review expired) or the rules cannot decide. For render paths: no waiting, no
    /// request.
    pub fn peek(&self, check: &AccessCheck) -> Option<AccessDecision> {
        self.cache.peek(check.namespace.as_deref())?.decide(check)
    }

    /// Whether `check` is allowed: from the cached (or freshly reviewed) rules when they can
    /// decide, else one `SelfSubjectAccessReview`.
    ///
    /// # Errors
    ///
    /// The access review's error when the rules could not decide and the review failed.
    pub async fn can_i(&self, check: &AccessCheck) -> OxiResult<AccessDecision> {
        if check.impersonate.is_none()
            && let Ok(rules) = self.cache.get(check.namespace.as_deref()).await
            && let Some(decision) = rules.decide(check)
        {
            return Ok(decision);
        }
        self.cache.port().can_i(check).await
    }

    /// Answers every check in order. Namespaces are reviewed once each (and at most
    /// [`MAX_IN_FLIGHT`] at a time); only what the rules cannot decide asks the server, again
    /// capped. A failed review is [`AccessDecision::Unknown`], never a deny.
    pub async fn can_i_many(&self, checks: &[AccessCheck]) -> Vec<AccessDecision> {
        let mut namespaces: Vec<Option<&str>> = Vec::new();
        for check in checks.iter().filter(|check| check.impersonate.is_none()) {
            let namespace = check.namespace.as_deref();
            if !namespaces.contains(&namespace) {
                namespaces.push(namespace);
            }
        }
        // Failures surface per check below, where `can_i` falls back to the server.
        stream::iter(namespaces)
            .for_each_concurrent(MAX_IN_FLIGHT, |namespace| async move {
                if let Err(error) = self.cache.get(namespace).await {
                    tracing::debug!(?namespace, %error, "rules review failed; asking per check");
                }
            })
            .await;
        stream::iter(checks)
            .map(|check| async move {
                match self.can_i(check).await {
                    Ok(decision) => decision,
                    Err(error) => AccessDecision::Unknown {
                        reason: error.message().to_owned(),
                    },
                }
            })
            .buffered(MAX_IN_FLIGHT)
            .collect()
            .await
    }

    /// Forgets every cached review: call it on connect and context switch.
    pub fn invalidate(&self) {
        self.cache.invalidate();
        self.cache.port().invalidate_all_rules();
    }

    /// Forgets the review of one namespace (`None`: the cluster-wide ask): call it when the
    /// user switches to it so the next ask is fresh.
    pub fn invalidate_namespace(&self, namespace: Option<&str>) {
        self.cache.invalidate_namespace(namespace);
        self.cache.port().invalidate_rules(namespace);
    }
}
