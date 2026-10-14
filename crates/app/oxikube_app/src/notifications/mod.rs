//! `NotificationService` (E14-S05): the in-app notification centre.
//!
//! Warnings should reach the user without them watching the events view. The service reads each
//! open cluster's `Warning` events from the [`EventService`](crate::events::EventService) (one
//! subscription per cluster, never a watch of its own) and keeps a bounded list of
//! [`Notification`]s with read / unread state, which the window shows as toasts, a status-bar
//! badge and the notification centre panel (`oxikube_workspace::notifications`), and which the
//! OS notifier receives when the user opted in ([`forward_to_os`]).
//!
//! ```text
//!  EventService (per cluster)            other sources (E14-S07): submit()
//!        │ EventsChanged (warnings, watched kinds)        │
//!        ▼                                                 │
//!  Tracker: only news (new events, higher counts)          │
//!        ▼                                                 ▼
//!  Rules: kinds to watch, reasons to ignore ──▶ Muted cluster? ──yes──▶ dropped
//!                                                    │ no
//!                                                    ▼
//!                        Limiter: group a burst per reason ("12 pods: BackOff")
//!                                                    ▼
//!                        Centre: dedupe by key (count up), read / unread, bounded
//!                                                    ▼
//!                        rate limit (per row, overall) ──▶ raised: toast + OS notifier
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | the service: submit, list, read state, mute, rules, subscribe | [`NotificationService`] (`service`) |
//! | a row, an input, an id | [`Notification`], [`NotificationInput`], [`NotificationId`], [`GroupKey`] (`model`) |
//! | which events notify, and the windows | [`NotificationRules`] (`rules`) |
//! | grouping and the raise limit | `limiter` |
//! | the list, dedupe, group rows, read state, bound | `centre` |
//! | which events are news (no backlog replayed on a relist) | `tracker` |
//! | one pump per open, unmuted cluster; the session follower | `driver` |
//! | the muted clusters on disk (`StatePort`, `notifications.muted`) | `mute` |
//! | what subscribers are told, coalesced per poll | [`NotificationsSubscription`], [`NotificationsChanged`] (`subscription`) |
//! | the four bus commands and their tool stubs | [`register_commands`] (`commands`) |
//! | the OS output | [`forward_to_os`] (`os`) |
//!
//! # Spam
//!
//! A burst of 1 000 Warning events in a second raises a handful of toasts: the first
//! [`NotificationRules::group_threshold`] objects of a reason get rows of their own and the rest
//! fold into one group row; a row raises at most once per [`NotificationRules::object_interval`];
//! at most [`NotificationRules::max_raised`] raise per [`NotificationRules::raise_window`]. Every
//! step is `O(1)` or `O(log n)` per event, and subscribers get one item per poll.
//!
//! # Content
//!
//! Titles and bodies are redacted (`oxikube_domain::redact`), flattened to one line and capped;
//! the OS gets them through `Notification::sanitised` again. Only the muted cluster ids are
//! written to disk. The list itself lives in memory: it does not survive a restart.

mod centre;
mod commands;
mod driver;
mod limiter;
mod model;
mod mute;
mod os;
mod rules;
mod service;
mod subscription;
mod tracker;

#[cfg(test)]
mod tests;

pub use commands::{COMMANDS, register_commands};
pub use model::{GroupKey, MAX_BODY_CHARS, Notification, NotificationId, NotificationInput};
pub use mute::MUTED_KEY;
pub use os::forward_to_os;
pub use rules::{ClusterRules, DEFAULT_WATCHED_KINDS, NotificationRules};
pub use service::NotificationService;
pub use subscription::{MAX_RAISED_PER_ITEM, NotificationsChanged, NotificationsSubscription};
