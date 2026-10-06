//! `oxikube_logs_ui` — layer: `ui`.
//!
//! Log viewer (single/aggregate/JSON), search, export, send-to-agent.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Modules
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`settings`] | E08-S01 | [`LogsSettings`]: the `logs` settings (`logs.buffer_lines`, the lines a session keeps) |
//! | [`runtime`] | E08-S01 | [`log_runtime`]: where `LogService` runs its stream tasks (the Tokio bridge) |
//! | [`follow`] | E08-S01 | [`follow_settings`]: a changed `logs.buffer_lines` reaches the open sessions at once |

pub mod follow;
pub mod runtime;
pub mod settings;
#[cfg(test)]
mod tests;

pub use follow::follow_settings;
pub use runtime::log_runtime;
pub use settings::{LogsContent, LogsSettings};
