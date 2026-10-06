//! The API server's `Warning:` headers, once each (E07-S10).
//!
//! The adapter publishes every warning header it sees on the connection's
//! [`WarningPort`]; a deprecated API warns on every list and watch restart, so showing each one
//! would bury the table in toasts. [`WarningLedger`] remembers what a session already showed, and
//! [`ResourceStore::warnings`](super::ResourceStore::warnings) yields only the first of each
//! distinct (code, text), however many tables of the session ask. The ledger lives and dies with
//! the store, so a reconnect (a new store) may show a warning again.

use std::collections::HashSet;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_ports::{ApiWarning, WarningPort};
use parking_lot::Mutex;

/// Most distinct warnings remembered per session; beyond it the ledger starts over (a server that
/// sends this many distinct warnings is a flood, and one more toast is the lesser evil).
const MAX_REMEMBERED: usize = 512;

/// Which warnings a session already showed.
#[derive(Default)]
pub(crate) struct WarningLedger {
    seen: Mutex<HashSet<ApiWarning>>,
}

impl WarningLedger {
    /// Records `warning`; `true` the first time it is seen.
    pub fn first_time(&self, warning: &ApiWarning) -> bool {
        let mut seen = self.seen.lock();
        if seen.len() >= MAX_REMEMBERED {
            seen.clear();
        }
        seen.insert(warning.clone())
    }
}

/// The deduplicated warning stream of one store: nothing without a port.
pub(crate) fn distinct(
    port: Option<&Arc<dyn WarningPort>>,
    ledger: Arc<WarningLedger>,
) -> BoxStream<'static, ApiWarning> {
    match port {
        Some(port) => port
            .subscribe()
            .filter(move |warning| futures::future::ready(ledger.first_time(warning)))
            .boxed(),
        None => futures::stream::empty().boxed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_warning_is_new_once_per_code_and_text() {
        let ledger = WarningLedger::default();
        let a = ApiWarning::new("v1 Foo is deprecated");
        assert!(ledger.first_time(&a));
        assert!(!ledger.first_time(&a));
        assert!(ledger.first_time(&ApiWarning::new("v1 Bar is deprecated")));
        let other_code = ApiWarning {
            code: 199,
            text: a.text,
        };
        assert!(
            ledger.first_time(&other_code),
            "the code is part of the identity"
        );
    }
}
