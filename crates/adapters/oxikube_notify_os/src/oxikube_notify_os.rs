//! `oxikube_notify_os` — layer: `adapters`.
//!
//! [`OsNotifier`], the `NotifierPort` adapter for OS desktop notifications (E14-S06).
//!
//! | Platform | Backend | Click |
//! |---|---|---|
//! | macOS | `NSUserNotificationCenter` through `mac-notification-sys` (what `notify-rust` wraps) | `wait_for_click`: the body click comes back as an activation |
//! | Linux, BSDs | freedesktop notifications over D-Bus (`notify-rust`, pure-Rust `zbus`) | the `default` action, offered only when the server advertises the `actions` capability |
//! | other | none: `set_enabled(true)` reports [`oxikube_ports::NotifierStatus::Unavailable`] | none |
//!
//! # Opt-in, startup and threads
//!
//! Nothing is initialised when the notifier is built: no D-Bus connection, no notification
//! centre, no permission prompt. [`oxikube_ports::NotifierPort::set_enabled`] with `true` is the user turning
//! OS notifications on, and the only place a backend connects. Every call into the OS runs on
//! a short-lived thread of its own (never the caller's, so never the UI thread); a thread that
//! waits for a click ends when the notification is clicked or dismissed, and at most
//! [`MAX_WAITING`] wait at once (further notifications are shown without click-through).
//! A click is reported to the handler given to `on_activated` on such a thread; the app layer
//! forwards it to the UI. This crate never calls GPUI.
//!
//! # macOS caveat
//!
//! The notification centre attributes a notification to a bundle identifier. A binary inside an
//! `Oxikube.app` bundle uses the bundle's own; an unbundled development binary (`cargo run`)
//! has none, so it shows as Terminal, may be suppressed by the OS, and a click may not come
//! back to the process. That degrades silently (a debug log), never panics. Linux needs a
//! running notification daemon; without one `set_enabled(true)` answers `Unavailable` and
//! `notify` returns the [`unavailable`](oxikube_ports::unavailable) error.
//!
//! Module map: [`payload`] (what is sent, sanitised and capped, no OS calls), [`backend`]
//! (the per-platform code behind one trait), [`notifier`] ([`OsNotifier`]).
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod backend;
pub mod notifier;
pub mod payload;

pub use backend::{Backend, BackendError, Response, Shown};
pub use notifier::{MAX_WAITING, OsNotifier};
pub use payload::{Payload, Urgency};
