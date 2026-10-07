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
//! | [`settings`] | E08-S01, S10 | [`LogsSettings`]: the `logs` settings (`buffer_lines`, `default_tail`, `wrap`, `timestamps`, `json_auto_detect` (JSON mode, E08-S05), `max_streams` (E08-S04), `reconnect_retries` (E08-S07)), per-cluster overrides, clamping |
//! | [`runtime`] | E08-S01 | [`log_runtime`]: where `LogService` runs its stream tasks (the Tokio bridge) |
//! | [`follow`] | E08-S01, S10 | [`follow_settings`]: a changed `logs.buffer_lines` (global or a cluster's) or `logs.max_streams` reaches the open sessions at once, off the UI thread |
//! | [`view`] | E08-S02 | [`LogView`]: a pod's log as a workspace tab (virtualised rows, wrap, timestamps, autoscroll with the "N new lines" pill, container selector, previous instance, tail / head / since presets, the `LogView` key context) |
//! | [`commands`] | E08-S02, E08-S04 | `pod::ViewLogs`, `workload::ViewLogs` and the `logs::*` commands on the bus, and [`LogViews`], which opens and drives the views of a window |
//! | [`view`] `recovery` | E08-S07 | after the stream stopped: the state row says why (reconnecting n/m, pod finished, replaced, deleted) and a strip offers [`Recovery`](view::Recovery): "Follow replacement" (`logs::FollowReplacement`) or "Reconnect" (`logs::Reconnect`) |
//! | [`view`] `aggregate` | E08-S04 | [`LogView::workload`]: a Deployment, StatefulSet, DaemonSet, ReplicaSet, Job or Service as one merged log (pod gutters coloured from the theme palette, the pod added / ended banner, "N more pods not streamed", the Sources menu) |
//! | [`export`] | E08-S06 | [`SaveDialog`]: what `logs::Save` would write (which lines, how many, the truncation note) before the user picks the file; [`suggested_file_name`] |
//! | [`search`] | E08-S03 | the search bar: regex with case and inverse toggles, highlight or filter mode, next / previous match with a count, the incremental match index over the ring buffer, and the per-session [`SearchMemory`] |
//! | [`view`] `agent` | E08-S09 | `logs::SendToAgent` (key `a`, the toolbar's "Send to agent"): the selected lines, else the lines on screen, as a context block with its source (cluster, namespace, pod or workload, container, time span), masked of secrets and queued in the app's `PendingContext` until the agent panel (E27) takes it |
//! | [`row_actions`] | E08-S02, E08-S04 | "View Logs" on a pod's row (and on a workload's or Service's) in the resource tables |
//!
//! A user reaches a log view from a pod's row in a resource table: its context menu (or the
//! palette's list for the selection) offers "View Logs", which sends `pod::ViewLogs`; the
//! handler hands the request to the window's [`LogViews`], which opens the view as a tab of the
//! pod's cluster tab. The viewer's keys are data in the keymap files (context `LogView`,
//! rebindable in `keymap.json`) and its defaults are the `logs` settings. An agent calling the
//! `k8s.pod_view_logs` tool lands in the same place. On a Deployment's, StatefulSet's,
//! DaemonSet's, ReplicaSet's, Job's or Service's row the same menu item sends
//! `workload::ViewLogs` (`k8s.workload_view_logs`) and the tab shows the merged log of all its
//! pods. When a pod's stream ends because a rollout replaced the pod, the tab offers "Follow
//! replacement" (`shift-r`) and switches to the pod that took over; a merged view follows the new
//! pods by itself.

pub mod commands;
pub mod export;
pub mod follow;
pub mod row_actions;
pub mod runtime;
pub mod search;
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
pub use search::{SearchMemory, SearchMode, SearchState};
pub use settings::{LogsContent, LogsSettings};
pub use view::{LogView, LogViewDeps, OpenLogs};
