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
//! | [`settings`] | E08-S01, S10 | [`LogsSettings`]: the `logs` settings (`buffer_lines`, `default_tail`, `wrap`, `timestamps`, `json_auto_detect`), per-cluster overrides, clamping |
//! | [`runtime`] | E08-S01 | [`log_runtime`]: where `LogService` runs its stream tasks (the Tokio bridge) |
//! | [`follow`] | E08-S01, S10 | [`follow_settings`]: a changed `logs.buffer_lines` (global or a cluster's) reaches the open sessions at once, off the UI thread |
//! | [`view`] | E08-S02 | [`LogView`]: a pod's log as a workspace tab (virtualised rows, wrap, timestamps, autoscroll with the "N new lines" pill, container selector, previous instance, tail / head / since presets, the `LogView` key context) |
//! | [`commands`] | E08-S02 | `pod::ViewLogs` and the `logs::*` commands on the bus, and [`LogViews`], which opens and drives the views of a window |
//! | [`row_actions`] | E08-S02 | "View Logs" on a pod's row in the resource tables |
//!
//! A user reaches a log view from a pod's row in a resource table: its context menu (or the
//! palette's list for the selection) offers "View Logs", which sends `pod::ViewLogs`; the
//! handler hands the request to the window's [`LogViews`], which opens the view as a tab of the
//! pod's cluster tab. The viewer's keys are data in the keymap files (context `LogView`,
//! rebindable in `keymap.json`) and its defaults are the `logs` settings. An agent calling the
//! `k8s.pod_view_logs` tool lands in the same place.

pub mod commands;
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
pub use follow::follow_settings;
pub use row_actions::log_row_actions;
pub use runtime::log_runtime;
pub use settings::{LogsContent, LogsSettings};
pub use view::{LogView, LogViewDeps, OpenLogs};
