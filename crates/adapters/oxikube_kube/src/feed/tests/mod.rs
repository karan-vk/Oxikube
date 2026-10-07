//! Feed tests against a scripted API server ([`server::FeedServer`]) behind a real
//! `kube::Client`, on a paused Tokio clock: idle time (coalescing windows, backoff, watch
//! timeouts) passes instantly and deterministically.

mod delivery;
mod metadata;
mod perf;
mod retry;
mod server;
mod sharing;
mod streaming;

use std::collections::BTreeMap;
use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{Delta, DeltaBatch};

use super::{FeedConfig, ReflectorFeed, StreamingLists};

/// Deterministic settings: paged lists, no jitter, short backoff.
fn config() -> FeedConfig {
    FeedConfig {
        streaming_lists: StreamingLists::Never,
        backoff_min: Duration::from_millis(100),
        backoff_max: Duration::from_secs(1),
        backoff_jitter: false,
        ..FeedConfig::default()
    }
}

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// The next item of `feed`; fails the test if none comes within ten (virtual) minutes.
async fn next(feed: &mut ReflectorFeed) -> OxiResult<DeltaBatch<Resource>> {
    tokio::time::timeout(Duration::from_secs(600), feed.next())
        .await
        .expect("a feed item in time")
        .expect("the feed is still open")
}

/// The next item, which must be a batch.
async fn next_batch(feed: &mut ReflectorFeed) -> DeltaBatch<Resource> {
    match next(feed).await {
        Ok(batch) => batch,
        Err(err) => panic!("expected a batch, got {err}"),
    }
}

/// What a consumer that folds the deltas holds: name to resource version.
#[derive(Default, Debug, PartialEq)]
struct Folded(BTreeMap<String, String>);

impl Folded {
    fn apply(&mut self, batch: DeltaBatch<Resource>) {
        for delta in batch {
            match delta {
                Delta::Restarted(all) => {
                    self.0 = all.iter().map(|r| (r.name().to_owned(), rv(r))).collect();
                }
                Delta::Applied(r) => {
                    self.0.insert(r.name().to_owned(), rv(&r));
                }
                Delta::Deleted(r) => {
                    self.0.remove(r.name());
                }
            }
        }
    }

    fn of(pairs: &[(&str, &str)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(n, v)| ((*n).into(), (*v).into()))
                .collect(),
        )
    }
}

fn rv(r: &Resource) -> String {
    r.meta
        .resource_version
        .as_deref()
        .unwrap_or_default()
        .to_owned()
}

/// `+name@rv`, `-name@rv` or `restart[names]` per delta, for order-sensitive assertions.
fn labels(batch: &DeltaBatch<Resource>) -> Vec<String> {
    batch
        .deltas
        .iter()
        .map(|delta| match delta {
            Delta::Applied(r) => format!("+{}@{}", r.name(), rv(r)),
            Delta::Deleted(r) => format!("-{}@{}", r.name(), rv(r)),
            Delta::Restarted(all) => {
                let mut names: Vec<_> = all.iter().map(|r| r.name().to_owned()).collect();
                names.sort();
                format!("restart[{}]", names.join(","))
            }
        })
        .collect()
}
