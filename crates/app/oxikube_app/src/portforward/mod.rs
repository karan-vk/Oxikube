//! [`PortForwardManager`] (E15-S03): port-forwards to pods and Services across clusters that
//! survive pod churn.
//!
//! * a registry of forwards keyed by [`ForwardId`], spanning clusters: [`start`], `stop`,
//!   `restart`, `remove`, `list` and a coalesced change subscription ([`ForwardChanges`]) for the
//!   panel; traffic counters are atomics sampled by `list`, never an event per byte;
//! * a target is a pod or a Service; a Service resolves through its EndpointSlices to a ready pod,
//!   preferring the pod used before ([`resolve`]);
//! * each forward is watched through the [`ResourceStore`](crate::ResourceStore) (ref-counted
//!   feeds, no polling) and restarted by a pure state machine ([`state`]): an immediate
//!   re-resolve when the pod goes away, backoff with jitter and a cap for repeated failures,
//!   permanent errors (forbidden, a busy or privileged port at the start, no such port) to
//!   `Failed`. The local port stays the same across reconnects;
//! * drops, recoveries and failures go to the notification centre ([`NotificationSourcePort`]),
//!   one row per forward;
//! * a start is refused on a read-only cluster unless [`ForwardConfig::allowed_in_readonly`];
//! * [`ForwardStarter`]: the start dialog's way through the bus (`portforward::Start` as the user);
//! * `portforward::Start` / `Stop` / `Restart` / `Remove` ([`register_commands`]) and
//!   `service::OpenInBrowser` (the manager is the [`ServicePortOpener`](crate::ServicePortOpener)).
//!
//! No gpui, no kube: the cluster side is a [`ForwardBackend`] ([`SessionBackend`] in the app),
//! supervisors run on a [`Spawner`](crate::store::Spawner) and are aborted when their forward
//! stops, is removed or the manager drops.
//!
//! [`start`]: PortForwardManager::start
//! [`NotificationSourcePort`]: oxikube_ports::NotificationSourcePort

mod alloc;
mod backend;
mod commands;
pub mod favourites;
mod manager;
mod model;
mod registry;
pub mod resolve;
mod starter;
pub mod state;
mod supervisor;
mod url;

#[cfg(test)]
mod tests;

pub use backend::{ForwardBackend, ObjectWatch, SessionBackend};
pub use commands::{COMMANDS, register_commands};
pub use favourites::{
    FavouriteForward, FavouriteId, FavouriteStore, PortForwardFavourites, RestoreReport,
};
pub use manager::{ForwardChanges, PortForwardManager, START_WAIT};
pub use model::{ForwardConfig, ForwardId, ForwardInfo, StartError};
pub use starter::{ForwardStarter, StartedForward};
pub use state::{Backoff, ForwardEvent, ForwardFailure, ForwardState};
