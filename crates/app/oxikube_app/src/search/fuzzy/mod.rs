//! [`FuzzyService`] (E11-S11): the one fuzzy ranking engine the command palette, the pickers and
//! the `:` bar's completions share, built on `nucleo-matcher`.
//!
//! | Piece | Where |
//! |---|---|
//! | [`FuzzyService::rank`], [`FuzzyService::rank_with`]: rank candidates for a query, best first, with the matched positions | `service` |
//! | the weights and `score::combine`: fuzzy score + exact / prefix / recents boosts, in one place | `score` |
//! | [`highlight`]: match positions as byte ranges for the views | `highlight` |
//! | [`QueryGeneration`]: latest-wins counter for asynchronous results | `generation` |
//!
//! # Use
//!
//! ```
//! use oxikube_app::search::fuzzy::FuzzyService;
//!
//! let service = FuzzyService::shared();
//! let titles = ["Workload Scale", "Workload Autoscale", "Pod Delete"];
//! let ranked = service.rank("scale", &titles, 10);
//! assert_eq!(ranked[0].index, 0, "a match at the start of a word beats one inside a word");
//! assert_eq!(ranked[0].positions, [9, 10, 11, 12, 13]);
//! ```
//!
//! # Rules
//!
//! - Every whitespace-separated word of the query must match (a subsequence of the candidate, in
//!   order); case is ignored unless the query has a capital; accents are normalised
//!   (`cafe` finds `Café`). The query is plain text: `^`, `$`, `'` and `!` mean themselves.
//! - A blank query matches every candidate, in the caller's order, except that recent
//!   candidates come first.
//! - The order is total and stable: higher score first, then the more recent candidate, then
//!   alphabetical (case-insensitive), then the caller's order, so equal scores never flicker
//!   between two keystrokes or two runs.
//! - `&self` throughout: the service keeps a small pool of matchers (each holds scratch buffers)
//!   that calls borrow for their duration, so one instance is shared by every surface and a call
//!   never waits for another's match. Nothing is allocated per candidate; the only allocations
//!   of a call are the compiled pattern and the returned vector.
//!
//! Plain Rust, no gpui: it runs on the foreground for a short list (2 000 commands rank in well
//! under a millisecond, `cargo bench -p oxikube_app --bench fuzzy_rank`) and on a background
//! executor for a long one; the caller decides, and drops stale results with a
//! [`QueryGeneration`].

mod generation;
pub mod highlight;
mod score;
mod service;
#[cfg(test)]
mod tests;

pub use generation::QueryGeneration;
pub use service::{FuzzyService, Match};
