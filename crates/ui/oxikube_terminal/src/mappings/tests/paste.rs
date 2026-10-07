//! Bracketed paste, the end-marker defence, multi-line detection and the dialog preview.

use crate::mappings::{PASTE_END, PASTE_START, encode_paste, is_multiline, preview};

#[test]
fn a_bracketed_paste_is_wrapped_and_left_alone() {
    let bytes = encode_paste("echo a\necho b", true);
    assert_eq!(&bytes[..], b"\x1b[200~echo a\necho b\x1b[201~");
    assert_eq!(&encode_paste("ls", true)[..], b"\x1b[200~ls\x1b[201~");
    assert_eq!(&encode_paste("", true)[..], b"\x1b[200~\x1b[201~");
}

#[test]
fn without_bracketed_paste_line_breaks_become_carriage_returns() {
    assert_eq!(&encode_paste("a\nb\r\nc\rd", false)[..], b"a\rb\rc\rd");
    assert_eq!(&encode_paste("ls -l", false)[..], b"ls -l");
    assert_eq!(&encode_paste("héllo ✓", false)[..], "héllo ✓".as_bytes());
}

#[test]
fn an_embedded_end_marker_cannot_end_the_bracket_early() {
    let hostile = format!("safe{PASTE_END}rm -rf ~\n");
    let bytes = encode_paste(&hostile, true);
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(text, format!("{PASTE_START}saferm -rf ~\n{PASTE_END}"));
    assert_eq!(
        text.matches(PASTE_END).count(),
        1,
        "only the final marker is left"
    );
}

#[test]
fn removing_a_marker_cannot_assemble_another() {
    // `ESC[2` + marker + `01~` collapses to a marker once the inner one is removed.
    let nested = "\x1b[2\x1b[201~01~tail";
    let text = String::from_utf8(encode_paste(nested, true).to_vec()).unwrap();
    assert_eq!(text, format!("{PASTE_START}tail{PASTE_END}"));
    let deep = "\x1b[2\x1b[2\x1b[201~01~01~x";
    let text = String::from_utf8(encode_paste(deep, true).to_vec()).unwrap();
    assert_eq!(text.matches(PASTE_END).count(), 1);
}

#[test]
fn multiline_means_any_line_break() {
    assert!(!is_multiline(""));
    assert!(!is_multiline("kubectl get pods -A"));
    assert!(is_multiline("a\nb"));
    assert!(is_multiline("a\r\nb"));
    assert!(is_multiline("a\rb"));
    assert!(
        is_multiline("rm -rf build\n"),
        "a trailing newline runs the line"
    );
}

#[test]
fn the_preview_shows_the_first_lines_and_counts_the_rest() {
    assert_eq!(preview("one\ntwo"), "one\ntwo");
    assert_eq!(preview("one\ntwo\n"), "one\ntwo");
    let ten: String = (1..=10).map(|n| format!("line {n}\n")).collect();
    let shown = preview(&ten);
    assert_eq!(
        shown,
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\n… and 4 more lines"
    );
    let seven: String = (1..=7).map(|n| format!("l{n}\n")).collect();
    assert!(preview(&seven).ends_with("… and 1 more line"));
}

#[test]
fn the_preview_cuts_long_lines_and_hides_control_characters() {
    let long = "x".repeat(300);
    let shown = preview(&long);
    assert_eq!(shown.chars().count(), 101);
    assert!(shown.ends_with('…'));
    assert_eq!(preview("a\x1b[31mb\x07\nc"), "a [31mb \nc");
    assert_eq!(preview("a\r\nb\rc"), "a\nb\nc");
}
