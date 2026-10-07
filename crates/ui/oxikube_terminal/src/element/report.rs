//! Mouse reporting to the process (E09-S06): while a program (htop, vim, tmux, less) has asked for
//! mouse events, clicks, drags, motion and the wheel go to it as escape sequences instead of
//! selecting text or scrolling the history.
//!
//! * Which events, and how they are encoded, is the process's choice ([`TerminalModes`]:
//!   1000 / 1002 / 1003, SGR 1006 preferred); [`crate::mappings::mouse`] does the encoding.
//! * **Shift bypasses it**: with Shift held a press selects text, a wheel scrolls the history, as
//!   in every terminal.
//! * A report is sent once per cell: moving inside a cell sends nothing.
//! * On the alternate screen without mouse reporting the wheel sends cursor keys (alternate
//!   scroll, `ESC [ ? 1007 h`, on by default), which is how `less` and man pages scroll.
//!
//! Reports go to the process only: they are not logged.

use bytes::Bytes;
use gpui::{
    App, Modifiers, MouseButton as GpuiButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window,
};

use super::mouse::Pointer;
use crate::grid::TerminalModes;
use crate::mappings::mouse::{MouseButton, MouseEvent, MouseKind, WheelDirection};
use crate::mappings::{encode_mouse, should_report, wheel_arrows};

/// The most wheel notches one scroll event sends (a hard flick must not flood the process).
const MAX_NOTCHES: usize = 20;

fn button(button: GpuiButton) -> Option<MouseButton> {
    match button {
        GpuiButton::Left => Some(MouseButton::Left),
        GpuiButton::Middle => Some(MouseButton::Middle),
        GpuiButton::Right => Some(MouseButton::Right),
        _ => None,
    }
}

impl Pointer {
    fn modes(&self) -> TerminalModes {
        self.state.0.borrow().snapshot.modes
    }

    /// Whether the process asked for mouse events and the user is not bypassing with Shift.
    pub(super) fn reporting(&self, modifiers: Modifiers) -> bool {
        !modifiers.shift && self.modes().intersects(TerminalModes::MOUSE_MODE)
    }

    fn send(&self, kind: MouseKind, row: usize, column: usize, modifiers: Modifiers, cx: &mut App) {
        let event = MouseEvent {
            kind,
            column,
            row,
            modifiers,
        };
        if let Some(bytes) = encode_mouse(&event, self.modes()) {
            let bytes = Bytes::copy_from_slice(bytes.as_bytes());
            self.terminal.read(cx).input(bytes);
        }
    }

    /// Reports a press of a button the protocol knows (others are ignored) and takes focus.
    pub(super) fn report_press(&self, event: &MouseDownEvent, window: &mut Window, cx: &mut App) {
        let Some(pressed) = button(event.button) else {
            return;
        };
        self.focus.focus(window, cx);
        let (row, column, _) = self.cell(event.position);
        {
            let mut inner = self.state.0.borrow_mut();
            inner.reported = Some(pressed);
            inner.report_cell = Some((row, column));
        }
        self.send(MouseKind::Press(pressed), row, column, event.modifiers, cx);
        cx.stop_propagation();
    }

    /// Reports the release of the button whose press was reported; `false` when it was not one.
    pub(super) fn report_release(&self, event: &MouseUpEvent, cx: &mut App) -> bool {
        let Some(released) = button(event.button) else {
            return false;
        };
        let (row, column, _) = self.cell(event.position);
        {
            let mut inner = self.state.0.borrow_mut();
            if inner.reported != Some(released) {
                return false;
            }
            inner.reported = None;
            inner.report_cell = None;
        }
        self.send(
            MouseKind::Release(released),
            row,
            column,
            event.modifiers,
            cx,
        );
        true
    }

    /// Reports a drag (a reported button is down) or bare motion, once per cell. Returns whether
    /// the pointer belongs to the process at the moment (so selection and link hover stay out).
    ///
    /// A drag keeps reporting when the pointer leaves the element (the button is the process's
    /// until it comes up); bare motion is only for the pointer over the element, since GPUI hands
    /// every mouse move in the window to every listener.
    pub(super) fn report_move(
        &self,
        event: &MouseMoveEvent,
        window: &Window,
        cx: &mut App,
    ) -> bool {
        let held = self.state.0.borrow().reported;
        if held.is_none() {
            if !self.reporting(event.modifiers) {
                return false;
            }
            if !self.hitbox.is_hovered(window) {
                // Back over the element it reports its cell again, even the one it left from.
                self.state.0.borrow_mut().report_cell = None;
                return false;
            }
        }
        let (row, column, _) = self.cell(event.position);
        {
            let mut inner = self.state.0.borrow_mut();
            if inner.report_cell == Some((row, column)) {
                return true;
            }
            inner.report_cell = Some((row, column));
        }
        let kind = held.map_or(MouseKind::Motion, MouseKind::Drag);
        if should_report(kind, self.modes()) {
            self.send(kind, row, column, event.modifiers, cx);
        }
        true
    }

    /// Handles a wheel of `lines` notches (positive: up) for the process: as mouse reports, or on
    /// the alternate screen as cursor keys. Returns whether it did; `false` leaves the wheel to
    /// the history.
    pub(super) fn report_wheel(
        &self,
        lines: i32,
        position: gpui::Point<gpui::Pixels>,
        modifiers: Modifiers,
        cx: &mut App,
    ) -> bool {
        let modes = self.modes();
        let up = lines > 0;
        let notches = (lines.unsigned_abs() as usize).min(MAX_NOTCHES);
        if self.reporting(modifiers) {
            let (row, column, _) = self.cell(position);
            let direction = if up {
                WheelDirection::Up
            } else {
                WheelDirection::Down
            };
            for _ in 0..notches {
                self.send(MouseKind::Wheel(direction), row, column, modifiers, cx);
            }
            return true;
        }
        if !modifiers.shift && modes.contains(TerminalModes::ALT_SCREEN) {
            // The alternate screen has no history: the wheel is the application's or nobody's.
            if modes.contains(TerminalModes::ALTERNATE_SCROLL) {
                let arrow = wheel_arrows(up, modes);
                for _ in 0..notches {
                    self.terminal
                        .read(cx)
                        .input(Bytes::from_static(arrow.as_bytes()));
                }
            }
            return true;
        }
        false
    }
}
