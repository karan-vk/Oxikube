//! `oxikube_portforward_ui` — layer: `ui`.
//!
//! Port-forward manager panel and start dialog.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`source`] | E15-S05 | [`ForwardSource`]: what the panel reads (the app's `PortForwardManager`, or a fake) |
//! | [`model`] | E15-S05 | [`Row`] and [`PanelModel`]: the forwards of every cluster as table rows (cluster, target, ports, status, traffic, uptime), ordered, filtered by cluster and text, with the selection; pure |
//! | [`panel`] | E15-S05 | [`PortForwardPanel`]: the dock panel (bottom), a virtualised table over the model, a toolbar of row actions, the `PortForwards` key context |
//! | [`commands`] | E15-S05 | `portforward::CopyUrl`, `OpenInBrowser` and `TogglePanel` on the bus ([`register_commands`], [`serve`]); `Stop`, `Restart` and `Remove` are the app's |
//!
//! The panel never talks to a cluster: it lists the manager's forwards, samples their traffic
//! counters once a second (atomics read by `list`, never an event per byte) and redraws at most once
//! per frame. Every action is a bus command, so a key, a button, the palette, `:pf` and an agent
//! run one behaviour.

pub mod commands;
pub mod model;
pub mod panel;
pub mod source;

pub use commands::{PortForwardUiSink, UiRequest, register_commands, serve};
pub use model::{PanelModel, Row, StatusKind};
pub use panel::{
    PANEL_NAME, PortForwardPanel, PortForwardPanelDeps, SAMPLE_INTERVAL, TogglePanel, install,
};
pub use source::ForwardSource;

#[cfg(test)]
mod tests;
