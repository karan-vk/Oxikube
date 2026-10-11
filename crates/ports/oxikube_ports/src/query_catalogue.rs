//! [`QueryCataloguePort`]: the PromQL for "CPU of this pod" on whatever stack a cluster has.
//!
//! Each Prometheus-compatible stack labels its series a little differently (ADR 0011), so a
//! graph does not write PromQL: it states a [`MetricRequest`] and the detected
//! [`MetricsProviderKind`] picks the query. Building a query is pure string work, so the port is
//! synchronous and free of I/O; running the query is the job of
//! [`PromqlPort`](crate::PromqlPort).
//!
//! # Adapter
//!
//! Implemented by `oxikube_prometheus` (`PromqlCatalogue`), one module per stack. The testkit
//! has `FakeQueryCataloguePort`.

use std::time::Duration;

use jiff::Timestamp;
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::metric_query::{
    INSTANT_RATE_WINDOW, MetricQuery, MetricQueryError, MetricRequest, steps_for,
};

use crate::promql::TimeRange;

/// Builds the PromQL of a [`MetricRequest`] for one provider.
///
/// # Effects
///
/// None: a pure function of its arguments.
///
/// # Errors
///
/// [`MetricQueryError`] when the request has no query (an unsupported scope and kind, an empty
/// name) or the window is zero. Every name that reaches the query text is escaped.
pub trait QueryCataloguePort: Send + Sync {
    /// The query for `request` on a `provider` stack, with every `rate()` over `rate_window`
    /// (use [`RangePlan::rate_window`] for a range query and [`INSTANT_RATE_WINDOW`] for an
    /// instant one).
    fn build(
        &self,
        provider: MetricsProviderKind,
        request: &MetricRequest,
        rate_window: Duration,
    ) -> Result<MetricQuery, MetricQueryError>;

    /// The query for the latest value ("now"): [`build`](Self::build) with
    /// [`INSTANT_RATE_WINDOW`].
    fn build_instant(
        &self,
        provider: MetricsProviderKind,
        request: &MetricRequest,
    ) -> Result<MetricQuery, MetricQueryError> {
        self.build(provider, request, INSTANT_RATE_WINDOW)
    }
}

/// A range query's interval and the `rate()` window that goes with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangePlan {
    /// The interval and step to pass to `PromqlPort::query_range`.
    pub range: TimeRange,
    /// The window to pass to [`QueryCataloguePort::build`].
    pub rate_window: Duration,
}

impl RangePlan {
    /// The plan for the `span` ending at `end`, with the step policy of
    /// [`steps_for`](oxikube_domain::metric_query::steps_for).
    pub fn ending_at(end: Timestamp, span: Duration) -> Self {
        let steps = steps_for(span);
        Self {
            range: TimeRange {
                start: end - span,
                end,
                step: steps.step,
            },
            rate_window: steps.rate_window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_spans_the_range_and_carries_the_policy() {
        let end = Timestamp::from_second(1_800_000_000).unwrap();
        let plan = RangePlan::ending_at(end, Duration::from_secs(6 * 3600));
        assert_eq!(plan.range.end, end);
        assert_eq!(plan.range.start, end - Duration::from_secs(6 * 3600));
        assert_eq!(plan.range.step, Duration::from_secs(120));
        assert_eq!(plan.rate_window, Duration::from_secs(240));
    }
}
