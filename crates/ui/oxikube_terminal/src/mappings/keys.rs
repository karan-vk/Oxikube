//! [`to_esc_str`]: a GPUI [`Keystroke`] to the bytes a terminal program expects.
//!
//! Plain text (letters, digits, punctuation, anything the IME composed) is *not* mapped: this
//! returns `None` and the keystroke goes on to the input handler, which writes the committed
//! text as UTF-8. Only keys that have no text, and text with Ctrl or Alt held, get a sequence.
//!
//! * Cursor keys, Home and End are `CSI X`, or `SS3 X` while the process has application cursor
//!   keys on (DECCKM). With a modifier they are always `CSI 1 ; m X`.
//! * Insert, Delete, Page Up/Down and F5-F12 are `CSI n ~` (`CSI n ; m ~` with a modifier);
//!   F1-F4 are `SS3 P..S` (`CSI 1 ; m P..S`).
//! * `m` is `1 + shift(1) + alt(2) + ctrl(4)`, xterm's modifier parameter.
//! * Ctrl-letter is the C0 control code (`ctrl-c` is `0x03`); Alt prefixes `ESC` ("meta").
//!   Alt on a character key is only meta when `option_as_meta` says so: on macOS Option composes
//!   characters, so by default it stays text.
//! * Anything with the platform key (cmd / super) is an application shortcut, never terminal
//!   input.

use std::borrow::Cow;

use gpui::{Keystroke, Modifiers};

use crate::grid::TerminalModes;

/// What the mapping needs to know besides the key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyMode {
    /// The process switched application cursor keys on (DECCKM).
    pub app_cursor: bool,
    /// Alt (Option) sends `ESC` + the key instead of composing a character.
    pub option_as_meta: bool,
}

impl KeyMode {
    /// The mode for a terminal whose process switched on `modes`, with the `option_as_meta`
    /// setting.
    pub fn new(modes: TerminalModes, option_as_meta: bool) -> Self {
        Self {
            app_cursor: modes.contains(TerminalModes::APP_CURSOR),
            option_as_meta,
        }
    }
}

/// How one non-text key is spelled: bare, with application cursor keys, and with each modifier
/// combination `2..=8`.
struct Sequences {
    plain: &'static str,
    application: &'static str,
    modified: [&'static str; 7],
}

macro_rules! modified {
    ($lead:literal, $tail:literal) => {
        [
            concat!("\x1b[", $lead, ";2", $tail),
            concat!("\x1b[", $lead, ";3", $tail),
            concat!("\x1b[", $lead, ";4", $tail),
            concat!("\x1b[", $lead, ";5", $tail),
            concat!("\x1b[", $lead, ";6", $tail),
            concat!("\x1b[", $lead, ";7", $tail),
            concat!("\x1b[", $lead, ";8", $tail),
        ]
    };
}

/// `ESC [ X` / `ESC O X` / `ESC [ 1 ; m X`.
macro_rules! cursor {
    ($final:literal) => {
        Sequences {
            plain: concat!("\x1b[", $final),
            application: concat!("\x1bO", $final),
            modified: modified!("1", $final),
        }
    };
}

/// `ESC [ n ~` / `ESC [ n ; m ~`.
macro_rules! tilde {
    ($n:literal) => {
        Sequences {
            plain: concat!("\x1b[", $n, "~"),
            application: concat!("\x1b[", $n, "~"),
            modified: modified!($n, "~"),
        }
    };
}

/// `ESC O X` / `ESC [ 1 ; m X` (F1-F4).
macro_rules! ss3 {
    ($final:literal) => {
        Sequences {
            plain: concat!("\x1bO", $final),
            application: concat!("\x1bO", $final),
            modified: modified!("1", $final),
        }
    };
}

static UP: Sequences = cursor!("A");
static DOWN: Sequences = cursor!("B");
static RIGHT: Sequences = cursor!("C");
static LEFT: Sequences = cursor!("D");
static HOME: Sequences = cursor!("H");
static END: Sequences = cursor!("F");
static INSERT: Sequences = tilde!("2");
static DELETE: Sequences = tilde!("3");
static PAGE_UP: Sequences = tilde!("5");
static PAGE_DOWN: Sequences = tilde!("6");
static FUNCTION: [Sequences; 12] = [
    ss3!("P"),
    ss3!("Q"),
    ss3!("R"),
    ss3!("S"),
    tilde!("15"),
    tilde!("17"),
    tilde!("18"),
    tilde!("19"),
    tilde!("20"),
    tilde!("21"),
    tilde!("23"),
    tilde!("24"),
];

/// The sequences of a non-text key by its GPUI name.
fn special(key: &str) -> Option<&'static Sequences> {
    Some(match key {
        "up" => &UP,
        "down" => &DOWN,
        "right" => &RIGHT,
        "left" => &LEFT,
        "home" => &HOME,
        "end" => &END,
        "insert" => &INSERT,
        "delete" => &DELETE,
        "pageup" => &PAGE_UP,
        "pagedown" => &PAGE_DOWN,
        _ => {
            let number: usize = key.strip_prefix('f')?.parse().ok()?;
            FUNCTION.get(number.checked_sub(1)?)?
        }
    })
}

/// xterm's modifier parameter minus nothing: 1 for none, 2 shift, 3 alt, 4 shift+alt, 5 ctrl,
/// 6 ctrl+shift, 7 ctrl+alt, 8 all three.
fn modifier_parameter(modifiers: &Modifiers) -> usize {
    1 + usize::from(modifiers.shift)
        + 2 * usize::from(modifiers.alt)
        + 4 * usize::from(modifiers.control)
}

const fn ascii_table() -> [u8; 128] {
    let mut table = [0; 128];
    let mut i = 0;
    while i < 128 {
        table[i] = i as u8;
        i += 1;
    }
    table
}

const fn meta_table() -> [[u8; 2]; 128] {
    let mut table = [[0x1b, 0]; 128];
    let mut i = 0;
    while i < 128 {
        table[i][1] = i as u8;
        i += 1;
    }
    table
}

static ASCII: [u8; 128] = ascii_table();
static META: [[u8; 2]; 128] = meta_table();

/// The one-byte string of an ASCII byte, without allocating.
fn ascii(byte: u8) -> Option<&'static str> {
    let index = usize::from(byte);
    std::str::from_utf8(ASCII.get(index..=index)?).ok()
}

/// `ESC` followed by an ASCII byte, without allocating.
fn meta(byte: u8) -> Option<&'static str> {
    std::str::from_utf8(META.get(usize::from(byte))?).ok()
}

/// The C0 control code Ctrl + `key` types (`ctrl-a` is 1, `ctrl-[` is ESC, `ctrl-?` is DEL, and
/// `ctrl--`, like `ctrl-/`, is 0x1f: readline's undo).
fn control_code(key: char) -> Option<u8> {
    Some(match key.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        '@' | '2' | ' ' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// The character an Alt chord types as meta: an ASCII `key_char` when the platform gave one
/// (it carries shift), otherwise the key itself (shifted when it is a letter). On macOS Option
/// composes `key_char` into something non-ASCII, which is why the key is the fallback.
fn meta_char(keystroke: &Keystroke, key: char) -> char {
    if let Some(typed) = keystroke.key_char.as_deref() {
        let mut chars = typed.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c.is_ascii()
            && !c.is_control()
        {
            return c;
        }
    }
    if keystroke.modifiers.shift && key.is_ascii_alphabetic() {
        key.to_ascii_uppercase()
    } else {
        key
    }
}

/// `ESC` + `c`.
fn escaped(c: char) -> Cow<'static, str> {
    match u8::try_from(c).ok().and_then(meta) {
        Some(sequence) => Cow::Borrowed(sequence),
        None => Cow::Owned(format!("\x1b{c}")),
    }
}

/// The bytes `keystroke` sends to the process, or `None` when it is not terminal input by itself
/// (plain text, which goes to the input handler; a bare modifier; an application shortcut).
///
/// ```
/// use gpui::Keystroke;
/// use oxikube_terminal::mappings::{KeyMode, to_esc_str};
///
/// let up = Keystroke::parse("up").unwrap();
/// assert_eq!(to_esc_str(&up, KeyMode::default()).as_deref(), Some("\x1b[A"));
/// let app = KeyMode { app_cursor: true, ..KeyMode::default() };
/// assert_eq!(to_esc_str(&up, app).as_deref(), Some("\x1bOA"));
/// let ctrl_c = Keystroke::parse("ctrl-c").unwrap();
/// assert_eq!(to_esc_str(&ctrl_c, KeyMode::default()).as_deref(), Some("\x03"));
/// ```
pub fn to_esc_str(keystroke: &Keystroke, mode: KeyMode) -> Option<Cow<'static, str>> {
    let modifiers = &keystroke.modifiers;
    if modifiers.platform {
        return None;
    }
    let key = keystroke.key.as_str();
    let parameter = modifier_parameter(modifiers);
    if let Some(sequences) = special(key) {
        return Some(Cow::Borrowed(match parameter {
            1 if mode.app_cursor => sequences.application,
            1 => sequences.plain,
            n => sequences.modified[n - 2],
        }));
    }
    let (alt, ctrl, shift) = (modifiers.alt, modifiers.control, modifiers.shift);
    let named = match key {
        "enter" => Some(if alt { "\x1b\r" } else { "\r" }),
        "escape" => Some(if alt { "\x1b\x1b" } else { "\x1b" }),
        "tab" if shift => Some(if alt { "\x1b\x1b[Z" } else { "\x1b[Z" }),
        // ctrl-tab switches tabs (a window shortcut), never a terminal key.
        "tab" if ctrl => return None,
        "tab" => Some(if alt { "\x1b\t" } else { "\t" }),
        "backspace" => Some(match (ctrl, alt) {
            (true, true) => "\x1b\x08",
            (true, false) => "\x08",
            (false, true) => "\x1b\x7f",
            (false, false) => "\x7f",
        }),
        "space" if ctrl => Some(if alt { "\x1b\x00" } else { "\x00" }),
        "space" if alt && mode.option_as_meta => Some("\x1b "),
        _ => None,
    };
    if let Some(sequence) = named {
        return Some(Cow::Borrowed(sequence));
    }
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if ctrl {
        let code = control_code(c)?;
        return Some(Cow::Borrowed(if alt { meta(code)? } else { ascii(code)? }));
    }
    if alt && mode.option_as_meta {
        return Some(escaped(meta_char(keystroke, c)));
    }
    None
}
