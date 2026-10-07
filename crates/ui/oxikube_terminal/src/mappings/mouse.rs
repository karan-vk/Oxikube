//! Mouse reporting: the escape sequences a click, drag, motion or wheel event becomes while the
//! process asked for them (htop, vim, tmux, less).
//!
//! The process picks which events it wants (modes 1000 click, 1002 drag, 1003 motion) and how
//! they are encoded (1006 SGR, 1005 UTF-8, otherwise the legacy single-byte form). SGR is the one
//! to prefer: it has no coordinate limit and tells press from release by the final byte.

use gpui::Modifiers;

use crate::grid::TerminalModes;

/// What the mouse did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    /// A button went down.
    Press(MouseButton),
    /// A button came up.
    Release(MouseButton),
    /// The pointer moved with a button held.
    Drag(MouseButton),
    /// The pointer moved with no button held.
    Motion,
    /// The wheel turned one notch.
    Wheel(WheelDirection),
}

/// The buttons a report names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The primary button.
    Left,
    /// The wheel button.
    Middle,
    /// The secondary button.
    Right,
}

/// Which way the wheel turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelDirection {
    /// Away from the user (scrolls up).
    Up,
    /// Towards the user (scrolls down).
    Down,
    /// Left.
    Left,
    /// Right.
    Right,
}

/// One mouse event on a cell of the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    /// What happened.
    pub kind: MouseKind,
    /// The viewport column, 0-based.
    pub column: usize,
    /// The viewport row, 0-based.
    pub row: usize,
    /// Shift, Alt and Ctrl add to the button code.
    pub modifiers: Modifiers,
}

/// An encoded report: up to 24 bytes on the stack.
#[derive(Debug, Clone, Copy)]
pub struct MouseBytes {
    buffer: [u8; 24],
    len: usize,
}

impl MouseBytes {
    fn new() -> Self {
        Self {
            buffer: [0; 24],
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        if let Some(slot) = self.buffer.get_mut(self.len) {
            *slot = byte;
            self.len += 1;
        }
    }

    fn push_str(&mut self, text: &str) {
        text.bytes().for_each(|byte| self.push(byte));
    }

    fn push_number(&mut self, mut value: usize) {
        let mut digits = [0u8; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        digits[start..].iter().for_each(|&digit| self.push(digit));
    }

    /// The report's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer[..self.len]
    }
}

/// Whether the process asked for `kind` under `modes`: clicks and the wheel in any mouse mode,
/// drags in 1002 and 1003, bare motion only in 1003.
pub fn should_report(kind: MouseKind, modes: TerminalModes) -> bool {
    if !modes.intersects(TerminalModes::MOUSE_MODE) {
        return false;
    }
    match kind {
        MouseKind::Press(_) | MouseKind::Release(_) | MouseKind::Wheel(_) => true,
        MouseKind::Drag(_) => {
            modes.intersects(TerminalModes::MOUSE_DRAG | TerminalModes::MOUSE_MOTION)
        }
        MouseKind::Motion => modes.contains(TerminalModes::MOUSE_MOTION),
    }
}

fn button_code(button: MouseButton) -> usize {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// xterm's button byte (before the +32 of the legacy forms): the button, +32 for a drag or
/// motion, +4 shift, +8 alt, +16 ctrl. The legacy forms cannot name the released button: they
/// send 3 ("a button came up"); SGR names it.
fn code(event: &MouseEvent, sgr: bool) -> usize {
    let base = match event.kind {
        MouseKind::Press(button) => button_code(button),
        MouseKind::Release(button) => {
            if sgr {
                button_code(button)
            } else {
                3
            }
        }
        MouseKind::Drag(button) => 32 + button_code(button),
        MouseKind::Motion => 32 + 3,
        MouseKind::Wheel(WheelDirection::Up) => 64,
        MouseKind::Wheel(WheelDirection::Down) => 65,
        MouseKind::Wheel(WheelDirection::Left) => 66,
        MouseKind::Wheel(WheelDirection::Right) => 67,
    };
    base + 4 * usize::from(event.modifiers.shift)
        + 8 * usize::from(event.modifiers.alt)
        + 16 * usize::from(event.modifiers.control)
}

/// Encodes `event` as the process asked (`modes`), or `None` when it did not ask for this kind
/// of event or the coordinates do not fit the legacy encoding (cells beyond 222; UTF-8: 2014).
///
/// ```
/// use gpui::Modifiers;
/// use oxikube_terminal::grid::TerminalModes;
/// use oxikube_terminal::mappings::mouse::{MouseButton, MouseEvent, MouseKind, encode_mouse};
///
/// let modes = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
/// let click = MouseEvent {
///     kind: MouseKind::Press(MouseButton::Left),
///     column: 9,
///     row: 4,
///     modifiers: Modifiers::none(),
/// };
/// assert_eq!(encode_mouse(&click, modes).unwrap().as_bytes(), b"\x1b[<0;10;5M");
/// ```
pub fn encode_mouse(event: &MouseEvent, modes: TerminalModes) -> Option<MouseBytes> {
    if !should_report(event.kind, modes) {
        return None;
    }
    let mut out = MouseBytes::new();
    if modes.contains(TerminalModes::SGR_MOUSE) {
        out.push_str("\x1b[<");
        out.push_number(code(event, true));
        out.push(b';');
        out.push_number(event.column + 1);
        out.push(b';');
        out.push_number(event.row + 1);
        let release = matches!(event.kind, MouseKind::Release(_));
        out.push(if release { b'm' } else { b'M' });
        return Some(out);
    }
    out.push_str("\x1b[M");
    let utf8 = modes.contains(TerminalModes::UTF8_MOUSE);
    for value in [code(event, false) + 32, event.column + 33, event.row + 33] {
        if utf8 {
            // Values up to 2047 take at most two UTF-8 bytes.
            let c = char::from_u32(u32::try_from(value).ok().filter(|&v| v <= 2047)?)?;
            let mut buffer = [0u8; 4];
            out.push_str(c.encode_utf8(&mut buffer));
        } else {
            out.push(u8::try_from(value).ok()?);
        }
    }
    Some(out)
}

/// The cursor-key press one wheel notch sends on the alternate screen without mouse reporting
/// (alternate scroll mode): Up or Down, as `SS3` while application cursor keys are on and `CSI`
/// otherwise. A static string; the caller repeats it per notch.
pub fn wheel_arrows(up: bool, modes: TerminalModes) -> &'static str {
    match (up, modes.contains(TerminalModes::APP_CURSOR)) {
        (true, true) => "\x1bOA",
        (true, false) => "\x1b[A",
        (false, true) => "\x1bOB",
        (false, false) => "\x1b[B",
    }
}
