//! The `:` jump bar (E11-S05): a command line for navigation, k9s's fastest habit. `:pods`,
//! `:deploy kube-system`, `:pod /re`, `:pod app=x`, `:ctx prod`, `:ns`, `:q`.
//!
//! The grammar, the parser, the plan (which `Command`s a line stands for), the history and the
//! completion are plain Rust in `oxikube_app::search::jump`; this module is the window's side.
//!
//! | File | Holds |
//! |---|---|
//! | `host.rs` | [`JumpHost`]: one per window, the actions that open the bar and run an earlier line (`[`, `]`, `-`) |
//! | `request.rs` | [`JumpRequest`], [`JumpSink`] and [`register_commands`]: the same doors from the command bus |
//! | `bar.rs` | [`JumpBar`]: the modal with the `JumpBar` key context and `jump_bar::Complete` (Tab) |
//! | `delegate.rs` | [`JumpDelegate`]: the picker's matches (what the word under the caret could be) and what Enter does |
//! | `sources.rs` | [`JumpSources`]: the live state it reads; [`LiveEnv`], the snapshot a line is planned against |
//! | `connect.rs` | the wait for a context that was not connected |
//! | `render.rs` | the problem line with the offending word underlined, the hints |
//!
//! # How a user gets here
//!
//! `:` in a resource table opens the bar (the binding is in the per-OS keymap files, context
//! `Table && !Editing`), and so do the `palette::OpenJump` command and the palette. Typing
//! completes (Tab takes the selected completion), Enter runs the line, Escape closes. `[` and `]`
//! in a table step back and forward through the lines run this session and `-` goes to the
//! previous view; `:-`, `:[` and `:]` do the same from the bar.
//!
//! # What Enter does
//!
//! A line is parsed on every keystroke (a problem so far is shown quietly) and planned on Enter.
//! A line that names something that does not exist stays in the bar with the word underlined and
//! close names offered. A good one closes the bar and its commands (`resource::OpenList`,
//! `namespace::Select`, `table::SetFilter`, `cluster::Select`, ...) go through the window's
//! `CommandDispatcher`, the path every key and button takes (non-negotiable 4); a jump to a context
//! whose session is not open connects it and does the rest once it is up.
//!
//! # Speed
//!
//! Opening lists the shown cluster's aliases without matching; a keystroke parses the line and
//! matches the word under the caret (inline up to a few hundred candidates, on the background
//! executor above that); the list is virtualised. The contexts and namespaces are read on the Tokio
//! bridge, never on the UI thread (`examples/jump_bench.rs` has the numbers).

mod bar;
mod connect;
mod delegate;
mod host;
mod render;
mod request;
mod sources;

#[cfg(test)]
mod tests;

pub use bar::JumpBar;
pub use connect::{ALIAS_WAIT, CONNECT_WAIT};
pub use delegate::JumpDelegate;
pub use host::JumpHost;
pub use request::{JumpRequest, JumpSink, register_commands};
pub use sources::{JumpSources, LiveEnv, Loaded};

use gpui::actions;

actions!(
    palette,
    [
        /// Open the `:` jump bar (`palette::OpenJump`), or close it when it is open.
        OpenJump,
    ]
);

actions!(
    jump,
    [
        /// Run the previous line of the jump history again (`jump::Back`, `[`).
        Back,
        /// Run the next line of the jump history again (`jump::Forward`, `]`).
        Forward,
        /// Go to the view before the current one, and back again (`jump::Last`, `-`).
        Last,
    ]
);

actions!(
    jump_bar,
    [
        /// Replace the word being typed by the selected completion (Tab). Bound in the `JumpBar`
        /// key context.
        Complete,
    ]
);

/// Registers the jump actions (`palette::OpenJump`, `jump::Back`, `jump::Forward`, `jump::Last`):
/// each acts on the jump bar of the active window. Call once at start-up, after the keymap; the
/// binary then installs a [`JumpHost`] per window.
pub fn init(cx: &mut gpui::App) {
    host::register_actions(cx);
}
