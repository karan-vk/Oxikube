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

pub mod settings;

pub use settings::{EventsContent, EventsSettings, follow_settings, follow_warning_window};
