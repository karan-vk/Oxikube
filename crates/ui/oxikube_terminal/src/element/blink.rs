//! The blinking cursor (E09-S11): the phase the element paints and the clock that flips it.
//!
//! The element never reads the time: it paints the cursor while [`Blink::shown`], and the host
//! view flips the phase every [`BLINK_INTERVAL`] ([`TerminalElementState::blink_tick`]) and
//! repaints. A terminal that is not on screen is not painted, so it stops asking for repaints.

use std::time::Duration;

use super::TerminalElementState;

/// Half a blink: how long the cursor stays on and off.
pub const BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// The cursor's blink phase and what the last frame knew about it.
#[derive(Debug)]
pub(super) struct Blink {
    /// The phase: the cursor is drawn when `true`.
    on: bool,
    /// The last painted frame showed a blinking cursor in a focused terminal.
    blinking: bool,
    /// A frame was painted since the last tick (so the terminal is on screen).
    painted: bool,
}

impl Default for Blink {
    fn default() -> Self {
        Self {
            on: true,
            blinking: false,
            painted: false,
        }
    }
}

impl Blink {
    /// Whether the cursor is drawn this frame.
    pub(super) fn shown(&self) -> bool {
        self.on
    }

    /// Records what this frame shows. A cursor that does not blink (or is not focused) is on.
    pub(super) fn painted(&mut self, blinking: bool) {
        self.blinking = blinking;
        self.painted = true;
        if !blinking {
            self.on = true;
        }
    }

    /// Back to "on" (a key was typed: the cursor must not vanish under the user's fingers).
    pub(super) fn reset(&mut self) {
        self.on = true;
    }
}

impl TerminalElementState {
    /// One step of the blink clock: flips the phase and returns whether the terminal needs a
    /// repaint (it showed a blinking cursor and was on screen since the last tick). The host
    /// calls it every [`BLINK_INTERVAL`] and notifies when it returns `true`.
    pub fn blink_tick(&self) -> bool {
        let mut inner = self.0.borrow_mut();
        let blink = &mut inner.blink;
        let repaint = blink.blinking && blink.painted;
        blink.painted = false;
        if repaint {
            blink.on = !blink.on;
        } else {
            blink.on = true;
        }
        repaint
    }

    /// Shows the cursor and restarts its blink: call it when the user types.
    pub fn blink_reset(&self) {
        self.0.borrow_mut().blink.reset();
    }

    /// Whether the cursor is in the "on" phase (tests).
    pub fn cursor_on(&self) -> bool {
        self.0.borrow().blink.shown()
    }
}
