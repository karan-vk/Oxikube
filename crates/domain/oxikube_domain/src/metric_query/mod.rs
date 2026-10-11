//! The vocabulary of history queries (E13-S06): *what* a graph asks for, independent of the
//! stack that answers.
//!
//! A detail panel asks for "CPU usage of pod `web-0` in `shop`" ([`MetricRequest`]); the query
//! catalogue of the detected [`MetricsProviderKind`](crate::MetricsProviderKind) turns that into
//! the PromQL text ([`MetricQuery`]) that stack understands. This module is pure data and pure
//! functions so the app and the views can build requests, ask which combinations exist
//! ([`MetricScope::supports`]) and pick the step of a range ([`steps_for`]) without touching an
//! adapter.
//!
//! | Item | Role |
//! |---|---|
//! | [`MetricScope`], [`MetricKind`], [`MetricUnit`] | the level a graph is about, what it measures and its unit |
//! | [`MetricTarget`], [`PodSelector`] | which object, with its namespace and name |
//! | [`MetricRequest`], [`LabelMatch`] | a target and a kind (plus extra matchers for shared stores) |
//! | [`MetricQuery`], [`MetricQueryError`] | the built query and why one could not be built |
//! | [`RangePreset`], [`steps_for`], [`RangeSteps`] | the step and `rate()` window policy of a range |

mod kind;
mod request;
mod step;
mod target;

pub use kind::{MetricKind, MetricScope, MetricUnit};
pub use request::{LabelMatch, MetricQuery, MetricQueryError, MetricRequest};
pub use step::{
    INSTANT_RATE_WINDOW, MAX_POINTS, MIN_RATE_WINDOW, MIN_STEP, RangePreset, RangeSteps,
    promql_duration, steps_for,
};
pub use target::{MetricTarget, PodSelector};

#[cfg(test)]
mod tests;
