//! Input encoding: what bytes a keystroke, a paste or a mouse event become on the wire (E09-S06).
//!
//! Pure functions over our own types, no GPUI entities and no I/O, so every key family x
//! modifier x mode combination is a table test. Written from the xterm control sequence
//! documentation (`ctlseqs`); Zed's terminal mapping table was read for the set of cases only and
//! nothing is copied from it, so there is no licence header to carry.
//!
//! | Module | What |
//! |---|---|
//! | [`keys`] | [`to_esc_str`]: a keystroke and the [`KeyMode`] -> the escape sequence (arrows, home/end, paging, insert/delete, F1-F12, tab, enter, escape, backspace, control codes, `CSI 1;mod X` modifier variants, DECCKM, alt as meta) |
//! | [`paste`] | [`encode_paste`] (bracketed paste, embedded end marker stripped), [`is_multiline`], [`preview`] for the confirmation dialog |
//! | [`mouse`] | [`encode_mouse`]: click, drag, motion and wheel reports in the SGR (1006), UTF-8 (1005) and legacy encodings, and [`wheel_arrows`] for the alternate-scroll mode |
//!
//! Nothing here allocates per keypress: sequences are `&'static str`s from tables (the owned
//! case is an Alt-prefixed non-ASCII character), mouse reports are written to a stack buffer.

pub mod keys;
pub mod mouse;
pub mod paste;

pub use keys::{KeyMode, to_esc_str};
pub use mouse::{MouseBytes, MouseEvent, MouseKind, encode_mouse, should_report, wheel_arrows};
pub use paste::{PASTE_END, PASTE_START, encode_paste, is_multiline, preview};

#[cfg(test)]
mod tests;
