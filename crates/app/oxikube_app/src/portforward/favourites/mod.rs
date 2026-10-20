//! Favourite port-forwards (E15-S06): forwards saved through the state store that start again
//! when their cluster connects.
//!
//! | Piece | Where |
//! |---|---|
//! | [`FavouriteForward`], [`FavouriteId`]: what is saved (a [`ForwardSpec`](oxikube_domain::portforward::ForwardSpec), a name, the auto-start switch) | `model` |
//! | [`FavouriteStore`]: the `port_forward_favourites` table of the [`StatePort`](oxikube_ports::StatePort) | `store` |
//! | [`PortForwardFavourites`]: add / remove / toggle, and `follow`, the loop that restores a cluster's favourites when its session becomes `Ready` | `service` |
//! | the restore: bounded concurrency, failures to notifications ([`RestoreReport`]) | `restore` |
//! | `portforward::FavouriteAdd`, `FavouriteRemove`, `FavouriteToggleAutoStart` ([`register_commands`]) | `commands` |
//!
//! # What is saved
//!
//! A cluster id, the target's resource reference, the remote port, the local port (`0` is
//! "Auto": a free port every time), the bind address, a display name and the switch. Nothing
//! from a kubeconfig, nothing secret (ADR 0010).
//!
//! # Restoring never gets in the way
//!
//! The follower reacts to the session's `Ready` transition by spawning the restore on the
//! runtime and going back to waiting, so the session is `Ready` (and the first frame drawn)
//! before any favourite starts. Favourites start [`RESTORE_CONCURRENCY`] at a time. One that
//! cannot start (a busy fixed port, no permission, a bad port name) or whose pod or Service is
//! gone is told to the notification centre with its suggestion and left alone; the others
//! start. A fixed local port that is busy is never swapped for another: only "Auto" favourites
//! pick a free port. On a read-only cluster with `portforward.allowed_in_readonly` off nothing
//! starts and one notification says why. A favourite that is forwarded already (the user's own
//! forward of the same port, or an earlier restore) is left alone, so restoring twice is harmless.

mod commands;
mod model;
mod restore;
mod service;
mod store;

#[cfg(test)]
mod tests;

pub use commands::{COMMANDS, register_commands};
pub use model::{FavouriteForward, FavouriteId};
pub use restore::{RESTORE_CONCURRENCY, RestoreReport};
pub use service::PortForwardFavourites;
pub use store::{FavouriteStore, TABLE};
