//! `oxikube_events_ui` — layer: `ui`.
//!
//! Cluster-wide events view and per-resource events tab.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`settings`] | E14-S02 | the `events` settings ([`EventsSettings`]: `events.buffer`, global and per cluster, and `events.warning_window`) and [`follow_settings`] and [`follow_warning_window`], which push them into the app's `EventService` and `WarningBadges` now and on every change |
//! | [`view`] | E14-S03 | [`EventsView`]: a cluster's events, live, filterable, in a virtualised table (`:ev`, the sidebar's Events heading, the palette's "Show Events") |
//! | [`model`] | E14-S03 | [`EventsModel`]: the rows kept in order as the changes arrive, the selection, the paused tail |
//! | [`filter`] | E14-S03 | [`EventsFilters`]: what the toolbar asks for, compiled into the service's `EventFilter` |
//! | [`persist`] | E14-S03 | the filters saved per cluster in the state store |
//! | [`export`] | E14-S03 | a row as text or JSON, the listed events as a JSON file, messages redacted |
//! | [`commands`] | E14-S03 | `events::Open`, `ToggleFaults`, `TogglePause`, `CopyRow`, `CopyRowJson`, `ExportJson` on the bus, and the window's [`EventsViews`] |
//!
//! The view reads only the app's `EventService` (one events feed per cluster); it never opens a
//! watch of its own.

pub mod commands;
pub mod export;
pub mod filter;
pub mod model;
pub mod persist;
pub mod settings;
pub mod view;

pub use commands::{
    EVENTS_COMMANDS, EventsCommandSink, EventsHost, EventsRequest, EventsViews, EventsViewsDeps,
    ViewChange, register_commands,
};
pub use filter::{EventsFilters, TypeChoice};
pub use model::{EventColumn, EventSort, EventsModel, RowSet, Tail};
pub use settings::{EventsContent, EventsSettings, follow_settings, follow_warning_window};
pub use view::{EventsView, EventsViewDeps, Field, ITEM_KEY};
