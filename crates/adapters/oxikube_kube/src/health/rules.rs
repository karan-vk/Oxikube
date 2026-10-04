//! `SelfSubjectRulesReview` per (context, namespace), with a short-lived cache.
//!
//! The review is the cheap way to learn what the current user may do in a namespace
//! (one POST instead of a `SelfSubjectAccessReview` per action). Answers are cached per
//! (context, namespace) so opening a tab, or switching between namespaces, does not
//! issue a request per namespace on every visit. Errors are never cached.
//!
//! The review is a `POST`, but it creates nothing and stores nothing; see the
//! [module docs](super) for why `MutationGuard` does not apply.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use k8s_openapi::api::authorization::v1::{
    ResourceRule, SelfSubjectRulesReview, SelfSubjectRulesReviewSpec, SubjectRulesReviewStatus,
};
use kube::api::PostParams;
use kube::{Api, Client};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ContextName;
use tokio::time::Instant;

use super::capabilities::{AccessRule, CapabilityReport, RulesSnapshot, capabilities_from_rules};
use crate::auth::{CredentialRefresh, classify_with};

/// Default time an answer stays fresh.
pub const DEFAULT_RULES_TTL: Duration = Duration::from_secs(60);

impl RulesSnapshot {
    /// Converts the server's review status (`None` becomes an empty, partial snapshot: the
    /// server returned no status, which is not evidence of "no access").
    pub fn from_status(status: Option<&SubjectRulesReviewStatus>) -> Self {
        let Some(status) = status else {
            return RulesSnapshot {
                incomplete: true,
                ..Default::default()
            };
        };
        RulesSnapshot {
            rules: status.resource_rules.iter().map(AccessRule::from).collect(),
            incomplete: status.incomplete,
            evaluation_error: status.evaluation_error.clone(),
        }
    }
}

impl From<&ResourceRule> for AccessRule {
    fn from(rule: &ResourceRule) -> Self {
        AccessRule {
            verbs: rule.verbs.clone(),
            api_groups: rule.api_groups.clone().unwrap_or_default(),
            resources: rule.resources.clone().unwrap_or_default(),
            resource_names: rule.resource_names.clone().unwrap_or_default(),
        }
    }
}

/// Asks the apiserver what the current user may do in `namespace`.
///
/// Read-only in effect: the review object is evaluated and returned, never persisted.
///
/// # Errors
///
/// The classified request failure (`Auth`, `Network`, `Forbidden`, ...).
pub async fn fetch_rules(
    client: &Client,
    namespace: &str,
    refresh: CredentialRefresh,
) -> OxiResult<RulesSnapshot> {
    let review = SelfSubjectRulesReview {
        metadata: Default::default(),
        spec: SelfSubjectRulesReviewSpec {
            namespace: Some(namespace.to_owned()),
        },
        status: None,
    };
    let api: Api<SelfSubjectRulesReview> = Api::all(client.clone());
    let answer = api
        .create(&PostParams::default(), &review)
        .await
        .map_err(|e| classify_with(&e, refresh))?;
    Ok(RulesSnapshot::from_status(answer.status.as_ref()))
}

#[derive(Debug)]
struct Entry {
    fetched: Instant,
    snapshot: Arc<RulesSnapshot>,
}

/// A TTL cache of [`RulesSnapshot`]s keyed by (context, namespace).
///
/// Time comes from `tokio::time::Instant`, so tests control it with paused time. Two
/// concurrent misses for the same key may both fetch; the later answer wins.
#[derive(Debug)]
pub struct RulesCache {
    ttl: Duration,
    entries: Mutex<HashMap<(ContextName, String), Entry>>,
}

impl Default for RulesCache {
    fn default() -> Self {
        Self::new(DEFAULT_RULES_TTL)
    }
}

impl RulesCache {
    /// A cache whose entries stay fresh for `ttl`.
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<(ContextName, String), Entry>> {
        // The map holds plain data; a panic elsewhere cannot leave it inconsistent.
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The cached snapshot if it is still fresh.
    pub fn get(&self, context: &ContextName, namespace: &str) -> Option<Arc<RulesSnapshot>> {
        let map = self.lock();
        let entry = map.get(&(context.clone(), namespace.to_owned()))?;
        (entry.fetched.elapsed() < self.ttl).then(|| entry.snapshot.clone())
    }

    /// Returns the fresh cached snapshot, or runs `fetch`, caches a successful answer and
    /// returns it. The lock is not held while `fetch` runs.
    ///
    /// # Errors
    ///
    /// Whatever `fetch` returns; failures are not cached.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        context: &ContextName,
        namespace: &str,
        fetch: F,
    ) -> OxiResult<Arc<RulesSnapshot>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = OxiResult<RulesSnapshot>>,
    {
        if let Some(hit) = self.get(context, namespace) {
            return Ok(hit);
        }
        let snapshot = Arc::new(fetch().await?);
        let mut map = self.lock();
        let ttl = self.ttl;
        map.retain(|_, e| e.fetched.elapsed() < ttl);
        map.insert(
            (context.clone(), namespace.to_owned()),
            Entry {
                fetched: Instant::now(),
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
    }

    /// Drops every entry of `context` (credentials changed, reconnect).
    pub fn invalidate_context(&self, context: &ContextName) {
        self.lock().retain(|(c, _), _| c != context);
    }

    /// Drops everything.
    pub fn clear(&self) {
        self.lock().clear();
    }
}

/// Capability levels for `namespace`, using and filling `cache`.
///
/// # Errors
///
/// The classified failure of the underlying review request.
pub async fn probe_capabilities(
    client: &Client,
    cache: &RulesCache,
    context: &ContextName,
    namespace: &str,
    refresh: CredentialRefresh,
) -> OxiResult<CapabilityReport> {
    let snapshot = cache
        .get_or_fetch(context, namespace, || {
            fetch_rules(client, namespace, refresh)
        })
        .await?;
    Ok(capabilities_from_rules(&snapshot))
}

#[cfg(test)]
mod tests;
