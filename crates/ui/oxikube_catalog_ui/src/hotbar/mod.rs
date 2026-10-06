//! The hotbar (E06-S04): a vertical strip at the window's left edge with the clusters the user
//! works with: every connected cluster and every favourite.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`HotbarModel`]: which tiles, in what order. Plain Rust, no GPUI |
//! | `store` | [`HotbarStore`]: the order the user dragged, in the state store |
//! | `view` | [`Hotbar`]: the GPUI entity, its loading, and its subscriptions |
//! | `render` | the strip and its tiles: colour dot, initials, state dot, tooltip, menu, drag |
//!
//! # Where the data comes from
//!
//! - **Connected clusters** come from the session manager and its update stream: a tile appears
//!   when a session leaves `Disconnected` and goes (unless the cluster is a favourite) when it
//!   comes back. Its colour is the session's colour and its name the session's title.
//! - **Favourites** come from the catalog ([`ClusterCatalog`](oxikube_app::ClusterCatalog)),
//!   read off the UI thread, and follow every later change through the catalog's favourite
//!   stream, so the star in the catalog and the hotbar never disagree.
//! - **The displayed cluster** comes from the window's [`ClusterTabs`](oxikube_workspace::ClusterTabs).
//!
//! # Commands
//!
//! A click is `cluster::Select` (or `cluster::Connect` for a favourite that is not connected),
//! "Close tab" is `cluster::CloseTab`, and the star in the menu is `cluster::ToggleFavourite`
//! (the story's "hotbar toggle favourite": the same command the catalog's star sends, so there
//! is one behaviour and one tool, `app.cluster_toggle_favourite`). The view never calls a
//! session or a port: it sends the command through its
//! [`CommandDispatcher`](oxikube_workspace::CommandDispatcher).
//!
//! # Placement
//!
//! The strip is not part of any layout: the binary hands it to the window's workspace with
//! `Workspace::set_strip`, so it stays at the left edge whatever the docks do.

mod model;
mod render;
mod store;
#[cfg(test)]
mod tests;
mod view;

pub use model::{HotbarEntry, HotbarModel, SessionLook};
pub use render::{HOTBAR_CONTEXT, HOTBAR_WIDTH};
pub use store::{HOTBAR_TABLE, HotbarStore};
pub use view::{Hotbar, HotbarDeps};
