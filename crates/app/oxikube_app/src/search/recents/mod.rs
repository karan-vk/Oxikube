//! Recents and history that survive a restart (E11-S11): the commands the palette ran lately and
//! the text typed into the `:` jump bar, kept through [`StatePort`](oxikube_ports::StatePort).
//!
//! | Piece | Where |
//! |---|---|
//! | [`StateRecents`]: the [`RecentsStore`](crate::RecentsStore) of the command palette, key `recents.commands` | `commands` |
//! | [`JumpRecents`]: the jump bar's history, one list per cluster, key `history.jump/<cluster>` | `jump` |
//! | [`RecentList`]: the ordered, deduplicated, capped list both are made of | `list` |
//! | `Writeback`: the dirty flag, the wake-up and the log-once for failed writes | `writeback` |
//!
//! # Scope
//!
//! Command recents are **global**: the same commands are handy in every cluster, and a command id
//! says nothing about one. Jump history is **per cluster** (`ClusterId`): `:deploy kube-system` is
//! useful where that namespace exists. Both are capped ([`RECENTS_CAPACITY`](crate::RECENTS_CAPACITY) 50,
//! [`JUMP_CAPACITY`] 100 per cluster) and deduplicated (a repeat moves to the front).
//!
//! # What is stored
//!
//! Command ids (`pod::Delete`) and jump text (`deploy kube-system`) only: no arguments, no
//! targets, no object data, nothing typed in any other input (non-negotiable 5). Jump text is
//! checked against the redaction patterns first: a line that looks like it carries a secret is
//! not remembered at all. The caller records a jump only after it ran.
//!
//! # Never in the way
//!
//! [`RecentsStore::record`](crate::RecentsStore::record) and [`JumpRecents::record`] only touch
//! memory and wake the writer; the palette never waits for the disk. The binary runs
//! [`StateRecents::run_writer`] (and the jump history's) on the runtime: it waits for a change,
//! pauses a moment so a burst becomes one write, then writes, and the app flushes once more on
//! quit. If the state database is unavailable, recents keep working in memory, a single redacted
//! line is logged, and the next change tries again.

mod commands;
mod jump;
mod list;
#[cfg(test)]
mod tests;
mod writeback;

pub use commands::{RECENTS_KEY, StateRecents};
pub use jump::{JUMP_CAPACITY, JUMP_TEXT_MAX_CHARS, JumpRecents};
pub use list::RecentList;
pub use writeback::DEBOUNCE;
