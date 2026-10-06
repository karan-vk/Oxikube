//! States and diagnostics (E07-S10): what a resource table says when it has no rows, or rows that
//! may be old, and how the user recovers.
//!
//! A blank table gives no hint whether it is loading, empty, forbidden or broken (k9s #4121,
//! #4170, #4106). Four states have their own icon and copy, derived by one pure function
//! ([`TableState::derive`]) from the feed's state, the row count and the active filter:
//!
//! | State | When | Offers |
//! |---|---|---|
//! | Loading | the feed is listing for the first time | skeleton rows, a spinner |
//! | Empty / filtered-empty | listed, none in the scope, or all hidden by the filter | the scope, the filter and a one-click clear |
//! | Forbidden / unauthorized | `403`, or `401` / an expired credential (an auth error, k9s #3730) | the verb and resource, an RBAC hint, Retry |
//! | Error / reconnecting | transport, timeout, unknown kind, watch budget, or a watch being reopened | a short message, details on expand, Retry |
//!
//! With rows, the same derivation marks them stale (a badge, with Retry when it helps) instead of
//! clearing them while a feed refreshes, reconnects or has stopped.
//!
//! Retry is a command (`resource::RetryFeed`): it restarts the feed through the store
//! (`Subscription::retry`, backoff stays in the store's driver) and keeps the old rows visible.
//! The API server's `Warning:` headers reach the table as [`ResourceTableEvent::ApiWarning`],
//! once per distinct text per session (the store dedupes), and the window shows them as a toast.
//!
//! | File | Holds |
//! |---|---|
//! | `state` | [`TableState`], [`Stale`]: the pure derivation |
//! | `copy` | [`StateLabels`], [`copy`](copy::copy): the words, redacted and bounded |
//! | `view` | the state view, the skeleton and the stale badge |
//! | `retry` | Retry, the filter clear and the details toggle on [`ResourceTable`] |
//! | `warnings` | following the store's warnings |
//!
//! [`ResourceTable`]: super::ResourceTable
//! [`ResourceTableEvent::ApiWarning`]: super::ResourceTableEvent::ApiWarning

mod copy;
mod retry;
mod state;
mod view;
mod warnings;

pub(super) use copy::scope_label;
pub use copy::{StateCopy, StateLabels, copy};
pub use state::{Stale, TableState};
pub(super) use view::{stale_badge, state_view};
pub(super) use warnings::poll_warnings;

#[cfg(test)]
mod tests;
