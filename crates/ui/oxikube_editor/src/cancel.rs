//! [`CancelToken`]: how a running parse or validation learns that its buffer has moved on.
//!
//! Dropping a GPUI `Task` stops its future at the next `await`, but a parse or a validation is one
//! synchronous call on the background executor: once started it would run to the end, so a stale
//! 5 MB run keeps a core busy while the next one waits. The editor gives every run a token and
//! cancels it on the next edit or when the editor is dropped ([`CancelOnDrop`]); the parser polls
//! it every few thousand events and the validator between objects and every few hundred nodes,
//! and both return [`Cancelled`] at the next poll.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A cancellation flag shared by the run and whoever may cancel it. Cloning shares the flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

/// The run stopped early because its [`CancelToken`] was cancelled; its partial work is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl CancelToken {
    /// A token nobody has cancelled yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks every run holding this token to stop at its next poll.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether [`cancel`](Self::cancel) was called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// `Err(Cancelled)` once cancelled, for `?` at a poll point.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Owns the token of the latest run and cancels it when dropped (or replaced), so a run already
/// on the background executor stops when its owner goes away, e.g. when the editor tab closes.
#[derive(Debug, Default)]
pub struct CancelOnDrop(CancelToken);

impl CancelOnDrop {
    /// A guard over `token`.
    #[must_use]
    pub fn new(token: CancelToken) -> Self {
        Self(token)
    }

    /// Cancels the guarded token now.
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let token = CancelToken::new();
        let run = token.clone();
        assert_eq!(run.check(), Ok(()));
        token.cancel();
        assert!(run.is_cancelled());
        assert_eq!(run.check(), Err(Cancelled));
    }

    #[test]
    fn dropping_or_replacing_the_guard_cancels_the_run() {
        let token = CancelToken::new();
        let guard = CancelOnDrop::new(token.clone());
        assert!(!token.is_cancelled());
        drop(guard);
        assert!(
            token.is_cancelled(),
            "a closed editor stops its running pass"
        );

        struct Owner {
            pass: CancelOnDrop,
        }
        let first = CancelToken::new();
        let mut owner = Owner {
            pass: CancelOnDrop::new(first.clone()),
        };
        let second = CancelToken::new();
        owner.pass = CancelOnDrop::new(second.clone());
        assert!(
            first.is_cancelled(),
            "a new pass cancels the one it replaces"
        );
        assert!(!second.is_cancelled());
        owner.pass.cancel();
        assert!(second.is_cancelled());
    }
}
