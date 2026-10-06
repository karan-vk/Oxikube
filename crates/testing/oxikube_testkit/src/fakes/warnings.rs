//! [`FakeWarningPort`]: the API server's `Warning:` headers, pushed by the test.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::stream::BoxStream;
use oxikube_ports::{ApiWarning, WarningPort};
use parking_lot::Mutex;

/// Fake `WarningPort`.
///
/// Every [`push`](Self::push) is delivered to the streams subscribed at that moment, like the
/// adapter's broadcast: nothing is replayed to a later subscriber. Streams whose receiver was
/// dropped are forgotten on the next push.
#[derive(Default)]
pub struct FakeWarningPort {
    subscribers: Mutex<Vec<UnboundedSender<ApiWarning>>>,
}

impl std::fmt::Debug for FakeWarningPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeWarningPort")
            .field("subscribers", &self.subscribers())
            .finish()
    }
}

impl FakeWarningPort {
    /// A port nobody subscribed to.
    pub fn new() -> Self {
        Self::default()
    }

    /// Delivers `warning` to every live subscriber.
    pub fn push(&self, warning: ApiWarning) {
        self.subscribers
            .lock()
            .retain(|tx| tx.unbounded_send(warning.clone()).is_ok());
    }

    /// Delivers a plain `299` warning with `text`.
    pub fn push_text(&self, text: &str) {
        self.push(ApiWarning::new(text));
    }

    /// How many subscribed streams are still alive.
    pub fn subscribers(&self) -> usize {
        self.subscribers
            .lock()
            .iter()
            .filter(|tx| !tx.is_closed())
            .count()
    }
}

impl WarningPort for FakeWarningPort {
    fn subscribe(&self) -> BoxStream<'static, ApiWarning> {
        let (tx, rx) = unbounded();
        self.subscribers.lock().push(tx);
        rx.boxed()
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    #[test]
    fn a_push_reaches_every_live_subscriber_and_is_not_replayed() {
        let port = FakeWarningPort::new();
        port.push_text("before anyone listens");
        let mut a = port.subscribe();
        let mut b = port.subscribe();
        port.push_text("v1 Foo is deprecated");
        assert_eq!(
            block_on(a.next()),
            Some(ApiWarning::new("v1 Foo is deprecated"))
        );
        assert_eq!(
            block_on(b.next()),
            Some(ApiWarning::new("v1 Foo is deprecated"))
        );
        drop(b);
        assert_eq!(port.subscribers(), 1);
    }
}
