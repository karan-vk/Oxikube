//! [`FakePromqlEndpointPort`]: a scripted wire-level PromQL client.
//!
//! Unlike the other fakes it queues `Result<_, PromqlError>`, not `OxiResult`, because the port's
//! error type is the PromQL taxonomy itself.

use std::collections::VecDeque;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_ports::{PromEndpoint, PromqlEndpointPort, PromqlError, PromqlSeries, TimeRange};
use parking_lot::Mutex;

use crate::script::CallLog;

/// One call made on a [`FakePromqlEndpointPort`].
#[derive(Debug, Clone, PartialEq)]
pub enum PromqlEndpointCall {
    /// `instant_query`.
    Instant {
        /// The endpoint's base URL (credentials are never recorded).
        base_url: String,
        /// PromQL text.
        query: String,
        /// Evaluation time.
        at: Option<Timestamp>,
    },
    /// `range_query`.
    Range {
        /// The endpoint's base URL.
        base_url: String,
        /// PromQL text.
        query: String,
        /// Range and step.
        range: TimeRange,
    },
}

/// Queued responses of a [`FakePromqlEndpointPort`], one queue for instant and range queries.
#[derive(Debug, Default)]
pub struct PromqlEndpointScripts {
    queue: Mutex<VecDeque<Result<Vec<PromqlSeries>, PromqlError>>>,
}

impl PromqlEndpointScripts {
    /// Queues a successful result.
    pub fn push_ok(&self, series: Vec<PromqlSeries>) -> &Self {
        self.queue.lock().push_back(Ok(series));
        self
    }

    /// Queues a failure.
    pub fn push_err(&self, error: PromqlError) -> &Self {
        self.queue.lock().push_back(Err(error));
        self
    }

    /// Number of responses still queued.
    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }

    /// `true` when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.queue.lock().is_empty()
    }
}

/// Fake `PromqlEndpointPort`. Responses are consumed in order; with nothing queued every call
/// fails with [`PromqlError::Unreachable`], the "no Prometheus" state.
#[derive(Debug, Default)]
pub struct FakePromqlEndpointPort {
    script: PromqlEndpointScripts,
    calls: CallLog<PromqlEndpointCall>,
}

fake_plumbing!(
    FakePromqlEndpointPort,
    PromqlEndpointScripts,
    PromqlEndpointCall
);

impl FakePromqlEndpointPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }

    fn next(&self) -> Result<Vec<PromqlSeries>, PromqlError> {
        self.script
            .queue
            .lock()
            .pop_front()
            .unwrap_or_else(|| Err(PromqlError::Unreachable("nothing scripted".into())))
    }
}

#[async_trait]
impl PromqlEndpointPort for FakePromqlEndpointPort {
    async fn instant_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        at: Option<Timestamp>,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
        self.calls.record(PromqlEndpointCall::Instant {
            base_url: endpoint.base_url.clone(),
            query: query.to_owned(),
            at,
        });
        self.next()
    }

    async fn range_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        range: TimeRange,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
        self.calls.record(PromqlEndpointCall::Range {
            base_url: endpoint.base_url.clone(),
            query: query.to_owned(),
            range,
        });
        self.next()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn scripted_results_are_consumed_in_order_and_calls_recorded() {
        futures::executor::block_on(async {
            let fake = FakePromqlEndpointPort::new();
            fake.script()
                .push_ok(Vec::new())
                .push_err(PromqlError::BadQuery {
                    message: "x".into(),
                });
            let endpoint = PromEndpoint::new("http://prom");

            assert!(
                fake.instant_query(&endpoint, "up", None)
                    .await
                    .unwrap()
                    .is_empty()
            );
            let range = TimeRange {
                start: Timestamp::UNIX_EPOCH,
                end: Timestamp::UNIX_EPOCH,
                step: Duration::from_secs(1),
            };
            let err = fake.range_query(&endpoint, "up", range).await.unwrap_err();
            assert!(err.is_query_error());
            assert!(fake.ping(&endpoint).await.unwrap_err().is_absent());
            assert_eq!(fake.recorded_calls().len(), 3);
        });
    }
}
