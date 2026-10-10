//! Opening resource tables and running the table commands (E07-S03).
//!
//! | File | Holds |
//! |---|---|
//! | `commands` | [`register_commands`]: `resource::Open`, `CopyName`, `SelectAll` on the bus, pushing [`ViewRequest`]s into a [`ResourceCommandSink`] |
//! | `controller` | [`ResourceViews`]: one per window, the kind view that opens a table in its cluster's tab for `resource::OpenList`, applies the table requests on the UI thread (tells tables to open a detail or select all, writes the clipboard) and resolves sidebar navigation through discovery |
//! | `crd` | the CRD navigation (`crd::OpenList`, `crd::OpenResources`: read the CRD, find the version to open, open its table) and the served versions each custom resource table offers its switcher (E07-S07) |
//! | `detail` | opening the detail drawer for `resource::Open`, pinning it as a tab for `resource::PinDetail`, copying a label for `resource::CopyLabel` (E07-S05) |
//! | `row_actions` | [`view_row_actions`]: "View YAML" and "Describe" in a row's menu and the palette (E11-S07) |
//! | `sidebar` | [`sidebar_navigation`]: the cluster-tab hook from the sidebar's `Navigate` to [`ResourceViews::navigate`] |
//!
//! A user reaches a table by clicking a kind in the cluster sidebar: the sidebar emits
//! `Navigate(Kind { group, resource })`, the hook asks [`ResourceViews::navigate`], which finds
//! the kind through discovery and dispatches `resource::OpenList`. That command is
//! [`navigate`](crate::navigate)'s (E07-S11): its handler hands the request back to the window,
//! which asks the registered [`KindViews`](crate::navigate::KindViews); [`ResourceViews`]
//! registered itself there and opens the table in the cluster's workspace
//! ([`ResourceViews::open_kind`]). A tile of the Workloads overview, the palette or an agent
//! dispatching `resource::OpenList` lands in the same place.

mod commands;
mod controller;
mod crd;
mod detail;
mod row_actions;
mod sidebar;

pub use commands::{RESOURCE_COMMANDS, ResourceCommandSink, ViewRequest, register_commands};
pub use controller::{ResourceViews, ResourceViewsDeps, find_kind};
pub use row_actions::{VIEW_DESCRIBE_ORDER, VIEW_YAML_ORDER, view_row_actions};
pub use sidebar::{ResourceViewsSlot, sidebar_navigation};
