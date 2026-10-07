//! What the process's bell does and the cursor's blink clock (E09-S11).
//!
//! `terminal.bell` is read when a bell rings: `none` ignores it, `visual` flashes the terminal
//! for [`FLASH_DURATION`], `audible` plays the system alert sound (a window call, so it happens
//! on the next frame). A process ringing in a loop is limited to one bell per
//! [`BELL_SPACING`]. The blink clock flips the cursor's phase every
//! [`BLINK_INTERVAL`](crate::element::BLINK_INTERVAL) and repaints only a terminal that shows a
//! blinking cursor on screen.

use std::time::{Duration, Instant};

use gpui::{Context, Task, Window};

use super::TerminalView;
use crate::element::BLINK_INTERVAL;
use crate::settings::{BellSetting, TerminalSettings};

/// How long the visual bell tints the terminal.
pub const FLASH_DURATION: Duration = Duration::from_millis(120);

/// The least time between two bells that do anything.
const BELL_SPACING: Duration = Duration::from_millis(200);

/// A terminal view's bell and blink state.
#[derive(Default)]
pub(super) struct Bell {
    /// The visual bell is showing.
    flashing: bool,
    /// How many times the system sound was played.
    sounded: usize,
    /// The system sound is due on the next frame.
    sound_due: bool,
    last_rang: Option<Instant>,
    /// Ends the flash; replaced by the next bell.
    flash: Option<Task<()>>,
    /// The blink clock; lives as long as the view's session.
    blink: Option<Task<()>>,
}

impl Bell {
    /// Whether the visual bell is showing.
    pub(super) fn flashing(&self) -> bool {
        self.flashing
    }

    /// Whether the system sound is due; clears it.
    pub(super) fn take_sound(&mut self) -> bool {
        std::mem::take(&mut self.sound_due)
    }

    /// Starts the blink clock (once per session).
    pub(super) fn start_blink(&mut self, cx: &mut Context<TerminalView>) {
        if self.blink.is_some() {
            return;
        }
        self.blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.element.blink_tick() {
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }
}

impl TerminalView {
    /// How many times the system alert sound was played for this terminal's bells.
    pub fn bells_sounded(&self) -> usize {
        self.bell.sounded
    }

    /// The process rang the bell: do what `terminal.bell` says.
    pub(super) fn ring_bell(&mut self, cx: &mut Context<Self>) {
        let setting = TerminalSettings::bell(cx);
        if setting == BellSetting::None {
            return;
        }
        let now = Instant::now();
        if self
            .bell
            .last_rang
            .is_some_and(|last| now.duration_since(last) < BELL_SPACING)
        {
            return;
        }
        self.bell.last_rang = Some(now);
        match setting {
            BellSetting::None => {}
            BellSetting::Audible => {
                self.bell.sound_due = true;
                cx.notify();
            }
            BellSetting::Visual => {
                self.bell.flashing = true;
                // Replacing the task cancels the previous flash's end; this one ends it.
                self.bell.flash = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(FLASH_DURATION).await;
                    this.update(cx, |this, cx| {
                        this.bell.flashing = false;
                        cx.notify();
                    })
                    .ok();
                }));
                cx.notify();
            }
        }
    }

    /// Plays the system sound when an audible bell is due. Called while the view renders: the
    /// sound is a window call.
    pub(super) fn play_due_bell(&mut self, window: &Window) {
        if self.bell.take_sound() {
            window.play_system_bell();
            self.bell.sounded += 1;
        }
    }
}
