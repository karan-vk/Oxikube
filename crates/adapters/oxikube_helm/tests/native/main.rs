//! The native release reader against the fake resource port: recorded helm 4.3.1 objects
//! (both storage drivers), the S01 testkit fixtures, synthetic records for every status,
//! decode failures, and the 500-release list budget.

mod detail;
mod errors;
mod listing;
mod recorded;
mod scale;
mod support;
