//! What the Workloads overview reads from outside the store (E12-S07).
//!
//! The counts and status buckets of the overview come from the store's running tallies
//! ([`WorkloadCounters`](crate::WorkloadCounters), `store::counts`); this module holds the other
//! input, the recent warnings, as a trait the Events feature (E14) implements.
//!
//! | File | Holds |
//! |---|---|
//! | `warnings` | [`RecentWarnings`], [`WarningRow`], [`NoRecentWarnings`] (what runs until E14 ships) and [`FixedWarnings`] (a list, for tests and previews) |

mod warnings;

#[cfg(test)]
mod tests;

pub use warnings::{FixedWarnings, NoRecentWarnings, RecentWarnings, WarningRow};
