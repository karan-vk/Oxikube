//! One refresh of one provider, as a task: the cache answer, detection, the scope fan-out and
//! the terminal event.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedSender;
use oxikube_ports::{CloudDiscoveryPort, CloudScope, CloudToolStatus, DiscoveredCluster};

use super::cache::{DiscoveryCache, Snapshot};
use super::event::{CloudDiscoveryEvent, DiscoveryConfig, RefreshMode};

/// Where a run sends its events. The terminal event is sent once, by the run or by
/// [`cancel`](super::CloudDiscoveryService::cancel), whichever ends it first.
#[derive(Clone)]
pub(super) struct RunEvents {
    tx: UnboundedSender<CloudDiscoveryEvent>,
    ended: Arc<AtomicBool>,
}

impl RunEvents {
    pub fn new(tx: UnboundedSender<CloudDiscoveryEvent>) -> Self {
        Self {
            tx,
            ended: Arc::default(),
        }
    }

    /// Sends a step. A dropped receiver is fine: the run still fills the cache.
    pub fn send(&self, event: CloudDiscoveryEvent) {
        let _ = self.tx.unbounded_send(event);
    }

    /// Sends `event` as the terminal event, unless the run already ended. Returns whether it
    /// was sent.
    pub fn end(&self, event: CloudDiscoveryEvent) -> bool {
        if self.ended.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.send(event);
        self.tx.close_channel();
        true
    }

    pub fn has_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }
}

/// What one run needs.
pub(super) struct Run {
    pub port: Arc<dyn CloudDiscoveryPort>,
    pub cache: DiscoveryCache,
    pub config: DiscoveryConfig,
    pub mode: RefreshMode,
    /// The scopes asked for; `None` lists the CLI's own.
    pub scopes: Option<Vec<CloudScope>>,
    pub events: RunEvents,
}

impl Run {
    pub async fn execute(self) {
        let provider = self.port.id();
        let cached = self.cache.load(provider).await;
        if !cached.is_empty() {
            let fresh = self.mode == RefreshMode::IfStale
                && cached.is_fresh(self.scopes.as_deref(), self.config.ttl, self.cache.now());
            self.send_cached(&cached, !fresh);
            if fresh {
                let clusters = clusters_of(&cached, &self.wanted(&cached));
                self.events.end(CloudDiscoveryEvent::Finished {
                    provider,
                    clusters,
                    failed_scopes: 0,
                });
                return;
            }
        }

        let detection = match self.port.detect().await {
            Ok(detection) => detection,
            Err(error) => {
                self.events
                    .end(CloudDiscoveryEvent::Failed { provider, error });
                return;
            }
        };
        let installed = detection.status != CloudToolStatus::NotInstalled;
        self.events
            .send(CloudDiscoveryEvent::Detected(detection.clone()));
        if !installed {
            self.cache.put_provider(&detection, Some(&[])).await;
            self.events.end(CloudDiscoveryEvent::Finished {
                provider,
                clusters: Vec::new(),
                failed_scopes: 0,
            });
            return;
        }

        let listed = self.scopes.is_none();
        let scopes = match &self.scopes {
            Some(scopes) => scopes.clone(),
            None => match self.port.scopes().await {
                Ok(scopes) => scopes,
                Err(error) => {
                    self.cache.put_provider(&detection, None).await;
                    self.events
                        .end(CloudDiscoveryEvent::Failed { provider, error });
                    return;
                }
            },
        };
        self.events.send(CloudDiscoveryEvent::Started {
            provider,
            scopes: scopes.clone(),
        });

        let (clusters, failed_scopes) = self.list_all(&scopes, &cached).await;
        self.cache
            .put_provider(&detection, listed.then_some(scopes.as_slice()))
            .await;
        if listed {
            self.cache.prune(provider, &scopes).await;
        }
        self.events.end(CloudDiscoveryEvent::Finished {
            provider,
            clusters,
            failed_scopes,
        });
    }

    /// Lists every scope, at most `max_parallel_scopes` at once. Dropping this future (a cancel)
    /// drops the listings in flight, which kills their CLI processes.
    async fn list_all(
        &self,
        scopes: &[CloudScope],
        cached: &Snapshot,
    ) -> (Vec<DiscoveredCluster>, usize) {
        let mut listings = futures::stream::iter(scopes.iter().cloned())
            .map(|scope| {
                let port = self.port.clone();
                async move {
                    let result = port.list_clusters(&scope).await;
                    (scope, result)
                }
            })
            .buffer_unordered(self.config.max_parallel_scopes.get());
        let mut clusters = Vec::new();
        let mut failed = 0;
        while let Some((scope, result)) = listings.next().await {
            match result {
                Ok(found) => {
                    self.cache.put_scope(&scope, &found).await;
                    clusters.extend(found.iter().cloned());
                    self.events.send(CloudDiscoveryEvent::ScopeDone {
                        scope,
                        clusters: found,
                    });
                }
                Err(error) => {
                    failed += 1;
                    clusters.extend(clusters_of(cached, std::slice::from_ref(&scope)));
                    self.events
                        .send(CloudDiscoveryEvent::ScopeFailed { scope, error });
                }
            }
        }
        sort(&mut clusters);
        (clusters, failed)
    }

    fn wanted(&self, cached: &Snapshot) -> Vec<CloudScope> {
        self.scopes.clone().unwrap_or_else(|| cached.known_scopes())
    }

    fn send_cached(&self, cached: &Snapshot, stale: bool) {
        let rows = cached.rows_of(&self.wanted(cached));
        self.events.send(CloudDiscoveryEvent::Cached {
            provider: self.port.id(),
            detection: cached.provider.as_ref().map(|p| p.detection.clone()),
            clusters: clusters_of(cached, &self.wanted(cached)),
            fetched_at: rows
                .iter()
                .map(|row| row.fetched_at)
                .min()
                .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok()),
            stale,
        });
    }
}

/// The cached clusters of `scopes`, sorted.
pub(super) fn clusters_of(cached: &Snapshot, scopes: &[CloudScope]) -> Vec<DiscoveredCluster> {
    let mut clusters: Vec<DiscoveredCluster> = cached
        .rows_of(scopes)
        .into_iter()
        .flat_map(|row| row.clusters.iter().cloned())
        .collect();
    sort(&mut clusters);
    clusters
}

/// Orders clusters by key, so every list a caller sees is stable.
fn sort(clusters: &mut [DiscoveredCluster]) {
    clusters.sort_by_cached_key(DiscoveredCluster::key);
}
