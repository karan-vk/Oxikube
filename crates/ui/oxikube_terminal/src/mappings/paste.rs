//! Pasting: the bytes a paste sends, and the check behind the multi-line confirmation.
//!
//! Pasted text goes to the process only; it is never stored or logged (non-negotiable 5).

use std::borrow::Cow;

use bytes::Bytes;

/// Starts a bracketed paste (`ESC [ 200 ~`).
pub const PASTE_START: &str = "\x1b[200~";
/// Ends a bracketed paste (`ESC [ 201 ~`).
pub const PASTE_END: &str = "\x1b[201~";

/// Lines the confirmation dialog shows at most.
const PREVIEW_LINES: usize = 6;
/// Characters of one preview line before it is cut with an ellipsis.
const PREVIEW_COLUMNS: usize = 100;

/// Removes every [`PASTE_END`] from `text`, repeating because removing one can join the
/// pieces around it into another (`ESC[2` + `ESC[201~` + `01~`). Text that tries to end the
/// bracket early could otherwise run the rest as typed commands.
fn strip_end_marker(text: &str) -> Cow<'_, str> {
    if !text.contains(PASTE_END) {
        return Cow::Borrowed(text);
    }
    let mut stripped = text.replace(PASTE_END, "");
    while stripped.contains(PASTE_END) {
        stripped = stripped.replace(PASTE_END, "");
    }
    Cow::Owned(stripped)
}

/// The bytes for pasting `text`.
///
/// With `bracketed` (the process turned bracketed paste on) the text is wrapped in
/// `ESC[200~ ... ESC[201~` and otherwise left alone, so the application sees real newlines and
/// can tell a paste from typing; an embedded end marker is stripped. Without it, line breaks
/// become carriage returns, which is what the Enter key sends.
pub fn encode_paste(text: &str, bracketed: bool) -> Bytes {
    if bracketed {
        let body = strip_end_marker(text);
        let mut out = Vec::with_capacity(PASTE_START.len() + body.len() + PASTE_END.len());
        out.extend_from_slice(PASTE_START.as_bytes());
        out.extend_from_slice(body.as_bytes());
        out.extend_from_slice(PASTE_END.as_bytes());
        Bytes::from(out)
    } else if text.contains(['\r', '\n']) {
        Bytes::from(text.replace("\r\n", "\r").replace('\n', "\r"))
    } else {
        Bytes::copy_from_slice(text.as_bytes())
    }
}

/// Whether `text` holds a line break, anywhere: a trailing one runs the line as it lands, so it
/// counts. Single-line text never needs confirming.
pub fn is_multiline(text: &str) -> bool {
    text.contains(['\r', '\n'])
}

/// The first lines of `text` for the confirmation dialog: at most six, long ones cut, control
/// characters shown as spaces, then "... and N more lines".
pub fn preview(text: &str) -> String {
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalised.split('\n').peekable();
    let mut out = String::new();
    for index in 0..PREVIEW_LINES {
        let Some(line) = lines.next() else { break };
        // A trailing newline leaves one empty last piece: not a line of its own.
        if line.is_empty() && lines.peek().is_none() && index > 0 {
            break;
        }
        if index > 0 {
            out.push('\n');
        }
        out.extend(
            line.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(PREVIEW_COLUMNS),
        );
        if line.chars().count() > PREVIEW_COLUMNS {
            out.push('…');
        }
    }
    let mut rest: Vec<&str> = lines.collect();
    // The empty piece after a trailing newline is not a line.
    if rest.last() == Some(&"") {
        rest.pop();
    }
    let more = rest.len();
    if more > 0 {
        out.push_str(&format!(
            "\n… and {more} more line{}",
            if more == 1 { "" } else { "s" }
        ));
    }
    out
}
