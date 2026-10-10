//! Cluster badges and the read-only / colour / preset controls (E06-S09).
//!
//! Making the production cluster impossible to miss, and impossible to change by accident:
//!
//! * [`ClusterMark`] / [`ClusterBadge`]: a colour dot and a lock, drawn the same way on a cluster
//!   tab ([`TabContent::cluster`](crate::TabContent::cluster)), a hotbar entry and the status bar
//!   ([`BadgeSurface`]). Colours come from the theme's status tokens ([`badge_colour`]), so they
//!   stay legible in light and dark.
//! * [`ClusterStatusItem`]: the status bar item for the active cluster; follows the session by
//!   itself ([`follow_session`], one redraw per change) and pulses at most once when a mutation
//!   is refused.
//! * [`cluster_menu`]: the "Read-only" toggle and the prod / staging / dev / none presets for a
//!   tab or hotbar context menu; each row is a [`Command`](oxikube_domain::command::Command).
//! * [`ClusterCommandRunner`]: dispatches those commands through the `CommandBus` and shows the
//!   outcome (toast, confirmation dialog, denial toast). Immediate commands run in the update
//!   that dispatched them, and [`SessionEcho`] hands the session updates they made to the views
//!   ([`observe_session_echo`]) in that same update (E05-P600).
//!
//! Nothing here enforces read-only mode. Hiding a button is a convenience; the check lives in
//! `oxikube_app::MutationGuard` and applies to every initiator.

mod colour;
mod echo;
mod follow;
mod mark;
mod menu;
mod runner;
mod status;
#[cfg(test)]
mod tests;

pub use colour::badge_colour;
pub use echo::{EchoItem, SessionEcho, observe_session_echo};
pub use follow::follow_session;
pub use mark::{BadgeSurface, ClusterBadge, ClusterMark};
pub use menu::{MenuEntry, cluster_menu, cluster_menu_entries};
pub use runner::{ClusterCommandRunner, denial_toast};
pub use status::ClusterStatusItem;
