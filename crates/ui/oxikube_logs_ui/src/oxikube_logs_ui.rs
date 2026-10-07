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
//! | [`view`] | E08-S02 | [`LogView`]: a pod's log as a workspace tab (virtualised rows, wrap, timestamps, autoscroll with the "N new lines" pill, container selector, previous instance, tail / head / since presets, the `LogView` key context) |
//! | [`commands`] | E08-S02 | `pod::ViewLogs` and the `logs::*` commands on the bus, and [`LogViews`], which opens and drives the views of a window |
//! | [`export`] | E08-S06 | [`SaveDialog`]: what `logs::Save` would write (which lines, how many, the truncation note) before the user picks the file; [`suggested_file_name`] |
//! | [`row_actions`] | E08-S02 | "View Logs" on a pod's row in the resource tables |
//!
//! A user reaches a log view from a pod's row in a resource table: its context menu (or the
//! palette's list for the selection) offers "View Logs", which sends `pod::ViewLogs`; the
//! handler hands the request to the window's [`LogViews`], which opens the view as a tab of the
//! pod's cluster tab. An agent calling the `k8s.pod_view_logs` tool lands in the same place.

pub mod commands;
pub mod export;
pub mod follow;
pub mod row_actions;
pub mod runtime;
pub mod settings;
pub mod view;

#[cfg(test)]
mod tests;

pub use commands::{
    LOG_COMMANDS, LogCommandSink, LogHost, LogRequest, LogViews, LogViewsDeps, ViewChange,
    register_commands,
};
pub use export::{SaveDialog, SaveOffer, SaveRequest, suggested_file_name};
pub use follow::follow_settings;
pub use row_actions::log_row_actions;
pub use runtime::log_runtime;
pub use settings::{LogsContent, LogsSettings};
pub use view::{LogView, LogViewDeps, OpenLogs};
