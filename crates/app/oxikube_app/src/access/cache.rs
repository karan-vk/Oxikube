//! [`RulesCache`]: rules reviews per namespace, with a TTL, single flight and invalidation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::access::AccessRules;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{AccessReviewPort, ClockPort};
use parking_lot::Mutex;

type Outcome = Result<Arc<AccessRules>, Arc<OxiError>>;
type Flight = Shared<BoxFuture<'static, Outcome>>;

struct Entry {
    rules: Arc<AccessRules>,
    fetched_at: Timestamp,
}

#[derive(Default)]
struct State {
    cluster_wide: Option<Entry>,
    by_namespace: HashMap<String, Entry>,
    flights: HashMap<Option<String>, Flight>,
    /// Bumped by every invalidation; a review that started before one is not stored.
    epoch: u64,
}

/// The cache behind [`AccessService`](super::AccessService).
pub(super) struct RulesCache {
    port: Arc<dyn AccessReviewPort>,
    clock: Arc<dyn ClockPort>,
    ttl: SignedDuration,
    state: Arc<Mutex<State>>,
}

impl RulesCache {
    pub(super) fn new(
        port: Arc<dyn AccessReviewPort>,
        clock: Arc<dyn ClockPort>,
        ttl: Duration,
    ) -> Self {
        Self {
            port,
            clock,
            ttl: SignedDuration::try_from(ttl).unwrap_or(SignedDuration::MAX),
            state: Arc::default(),
        }
    }

    pub(super) fn port(&self) -> &Arc<dyn AccessReviewPort> {
        &self.port
    }

    fn is_fresh(&self, entry: &Entry, now: Timestamp) -> bool {
        now.duration_since(entry.fetched_at) < self.ttl
    }

    /// The fresh cached rules of `namespace`, without waiting.
    pub(super) fn peek(&self, namespace: Option<&str>) -> Option<Arc<AccessRules>> {
        let now = self.clock.now();
        let state = self.state.lock();
        let entry = match namespace {
            Some(name) => state.by_namespace.get(name),
            None => state.cluster_wide.as_ref(),
        }?;
        self.is_fresh(entry, now).then(|| entry.rules.clone())
    }

    /// The rules of `namespace`: cached when fresh, else one shared review.
    pub(super) async fn get(&self, namespace: Option<&str>) -> OxiResult<Arc<AccessRules>> {
        let flight = {
            let now = self.clock.now();
            let mut state = self.state.lock();
            let cached = match namespace {
                Some(name) => state.by_namespace.get(name),
                None => state.cluster_wide.as_ref(),
            };
            if let Some(entry) = cached.filter(|entry| self.is_fresh(entry, now)) {
                return Ok(entry.rules.clone());
            }
            let key = namespace.map(str::to_owned);
            match state.flights.get(&key) {
                Some(flight) => flight.clone(),
                None => {
                    let flight = self.start(key.clone(), state.epoch);
                    state.flights.insert(key, flight.clone());
                    flight
                }
            }
        };
        flight.await.map_err(|err| {
            OxiError::new(err.kind(), err.message().to_owned()).with_retryable(err.is_retryable())
        })
    }

    fn start(&self, namespace: Option<String>, epoch: u64) -> Flight {
        let port = self.port.clone();
        let clock = self.clock.clone();
        let state = self.state.clone();
        async move {
            let answer = port.rules(namespace.as_deref()).await;
            let mut state = state.lock();
            if state.epoch == epoch {
                state.flights.remove(&namespace);
            }
            let mut rules = answer.map_err(Arc::new)?;
            if namespace.is_none() {
                // The cluster-wide ask sees one probe namespace's Roles only.
                rules.namespaces_unseen = true;
            }
            let rules = Arc::new(rules);
            if state.epoch == epoch {
                let entry = Entry {
                    rules: rules.clone(),
                    fetched_at: clock.now(),
                };
                match namespace {
                    Some(name) => {
                        state.by_namespace.insert(name, entry);
                    }
                    None => state.cluster_wide = Some(entry),
                }
            }
            Ok(rules)
        }
        .boxed()
        .shared()
    }

    /// Forgets every namespace (connect, context switch).
    pub(super) fn invalidate(&self) {
        let mut state = self.state.lock();
        state.epoch += 1;
        state.cluster_wide = None;
        state.by_namespace.clear();
        state.flights.clear();
    }

    /// Forgets one namespace (`None` is the cluster-wide ask).
    pub(super) fn invalidate_namespace(&self, namespace: Option<&str>) {
        let mut state = self.state.lock();
        state.epoch += 1;
        match namespace {
            Some(name) => {
                state.by_namespace.remove(name);
            }
            None => state.cluster_wide = None,
        }
        state.flights.clear();
    }

    /// How many namespaces hold a cached review (expired ones included).
    pub(super) fn len(&self) -> usize {
        let state = self.state.lock();
        state.by_namespace.len() + usize::from(state.cluster_wide.is_some())
    }
}
