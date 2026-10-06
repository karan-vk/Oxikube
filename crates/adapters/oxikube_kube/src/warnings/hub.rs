//! [`WarningHub`]: one broadcast channel per kubeconfig context.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_domain::ids::ContextName;
use oxikube_ports::{ApiWarning, WarningPort};
use parking_lot::Mutex;
use tokio::sync::broadcast;

/// How many warnings a slow subscriber may fall behind before it loses the oldest.
const CAPACITY: usize = 64;

/// The channels of every context's warnings. Cheap to clone; clones share the channels.
#[derive(Clone, Default)]
pub struct WarningHub {
    channels: Arc<Mutex<HashMap<ContextName, broadcast::Sender<ApiWarning>>>>,
}

impl std::fmt::Debug for WarningHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarningHub")
            .field("contexts", &self.channels.lock().len())
            .finish()
    }
}

static GLOBAL: LazyLock<WarningHub> = LazyLock::new(WarningHub::default);

impl WarningHub {
    /// The process-wide hub the pool's clients publish to and the connector reads.
    pub fn global() -> &'static WarningHub {
        &GLOBAL
    }

    fn sender(&self, context: &ContextName) -> broadcast::Sender<ApiWarning> {
        self.channels
            .lock()
            .entry(context.clone())
            .or_insert_with(|| broadcast::channel(CAPACITY).0)
            .clone()
    }

    /// Where a client of `context` publishes.
    pub fn sink(&self, context: &ContextName) -> WarningSink {
        WarningSink {
            tx: self.sender(context),
        }
    }

    /// `context`'s warnings as the port a connection exposes.
    pub fn port(&self, context: &ContextName) -> HubPort {
        HubPort {
            tx: self.sender(context),
        }
    }
}

/// Publishes warnings of one context. Cheap to clone; never blocks.
#[derive(Clone)]
pub struct WarningSink {
    tx: broadcast::Sender<ApiWarning>,
}

impl WarningSink {
    /// Publishes `warning` to the current subscribers (none is fine: nothing is kept).
    pub fn publish(&self, warning: ApiWarning) {
        // Errs only when nobody listens.
        self.tx.send(warning).ok();
    }
}

impl std::fmt::Debug for WarningSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarningSink")
            .field("subscribers", &self.tx.receiver_count())
            .finish()
    }
}

/// The [`WarningPort`] over one context's channel.
#[derive(Clone)]
pub struct HubPort {
    tx: broadcast::Sender<ApiWarning>,
}

impl std::fmt::Debug for HubPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubPort")
            .field("subscribers", &self.tx.receiver_count())
            .finish()
    }
}

impl WarningPort for HubPort {
    fn subscribe(&self) -> BoxStream<'static, ApiWarning> {
        let rx = self.tx.subscribe();
        futures::stream::unfold(rx, |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(warning) => return Some((warning, rx)),
                    // The subscriber fell behind: carry on with the newest.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(name: &str) -> ContextName {
        ContextName::new(name)
    }

    #[tokio::test]
    async fn a_context_publishes_to_its_own_subscribers_only() {
        let hub = WarningHub::default();
        let mut mine = hub.port(&ctx("a")).subscribe();
        let mut other = hub.port(&ctx("b")).subscribe();
        hub.sink(&ctx("a")).publish(ApiWarning::new("deprecated"));
        assert_eq!(mine.next().await, Some(ApiWarning::new("deprecated")));
        hub.sink(&ctx("b")).publish(ApiWarning::new("other"));
        assert_eq!(other.next().await, Some(ApiWarning::new("other")));
    }

    #[tokio::test]
    async fn publishing_without_subscribers_is_fine_and_nothing_is_replayed() {
        let hub = WarningHub::default();
        hub.sink(&ctx("a")).publish(ApiWarning::new("early"));
        let mut late = hub.port(&ctx("a")).subscribe();
        hub.sink(&ctx("a")).publish(ApiWarning::new("later"));
        assert_eq!(late.next().await, Some(ApiWarning::new("later")));
    }

    #[tokio::test]
    async fn a_slow_subscriber_loses_the_oldest_and_keeps_going() {
        let hub = WarningHub::default();
        let mut slow = hub.port(&ctx("a")).subscribe();
        let sink = hub.sink(&ctx("a"));
        for i in 0..(CAPACITY * 2) {
            sink.publish(ApiWarning::new(format!("w{i}")));
        }
        let first = slow.next().await.expect("a warning");
        assert!(first.text != "w0", "the oldest were dropped: {first:?}");
    }
}
