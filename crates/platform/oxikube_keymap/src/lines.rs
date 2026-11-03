//! Where things are in a `keymap.json`: the 1-based line and column and the byte span of each
//! section, its `context` and each of its bindings, so a diagnostic can say `keymap.json:12` and
//! an editor can underline the text.
//!
//! `serde_json` parses to values without positions, and by the time a binding is rejected the
//! text has long been parsed. This is a second, tiny pass over text that already parsed as JSON
//! with comments: it skips comments and strings, follows the nesting, and records the lines of
//! the keys it cares about. It never fails; on text it cannot follow it just knows fewer lines.

use std::collections::HashMap;
use std::ops::Range;

/// A place in the text: where a token starts and the bytes it covers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Location {
    /// 1-based line the token starts on.
    pub line: usize,
    /// 1-based column (in characters) the token starts at.
    pub column: usize,
    /// The bytes of the token (a string literal includes its quotes).
    pub span: Range<usize>,
}

/// A binding key written twice in one section's `bindings`; JSON maps keep only the last, so the
/// parsed sections cannot show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateKey {
    /// Index of the section.
    pub section: usize,
    /// The keystrokes as written (unescaped).
    pub keystrokes: String,
    /// The earlier occurrence, which is overridden.
    pub first: Location,
    /// The later occurrence, which wins.
    pub second: Location,
}

/// The locations of one section (an element of the top-level list).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SectionLines {
    start: Location,
    context: Option<Location>,
    /// Binding key (as written, unescaped) to where it is. A key written twice keeps the later
    /// one, which is the binding that wins.
    bindings: HashMap<String, Location>,
}

/// The locations of the sections of one keymap file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceLines {
    sections: Vec<SectionLines>,
    duplicates: Vec<DuplicateKey>,
}

impl SourceLines {
    /// The line the `index`th section starts on.
    pub fn section(&self, index: usize) -> Option<usize> {
        self.section_location(index).map(|at| at.line)
    }

    /// The line of the `context` of the `index`th section (its value when a string, else the key).
    pub fn context(&self, index: usize) -> Option<usize> {
        self.context_location(index).map(|at| at.line)
    }

    /// The line of the binding `keystrokes` in the `index`th section.
    pub fn binding(&self, index: usize, keystrokes: &str) -> Option<usize> {
        self.binding_location(index, keystrokes).map(|at| at.line)
    }

    /// Where the `index`th section starts (its opening brace).
    pub fn section_location(&self, index: usize) -> Option<&Location> {
        self.sections.get(index).map(|section| &section.start)
    }

    /// Where the `context` of the `index`th section is.
    pub fn context_location(&self, index: usize) -> Option<&Location> {
        self.sections.get(index)?.context.as_ref()
    }

    /// Where the key `keystrokes` of the `index`th section is.
    pub fn binding_location(&self, index: usize, keystrokes: &str) -> Option<&Location> {
        self.sections.get(index)?.bindings.get(keystrokes)
    }

    /// Keystrokes written twice in one section, in file order.
    pub fn duplicates(&self) -> &[DuplicateKey] {
        &self.duplicates
    }

    /// The 1-based line and column of byte `offset` of `text`, clamped to the end.
    pub fn position(text: &str, offset: usize) -> (usize, usize) {
        let mut end = offset.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let before = &text[..end];
        let line = before.matches('\n').count() + 1;
        let start = before.rfind('\n').map_or(0, |newline| newline + 1);
        (line, before[start..].chars().count() + 1)
    }

    /// The byte offset of 1-based `line` and `column` (in characters) of `text`, clamped to the
    /// end of the text.
    pub fn offset(text: &str, line: usize, column: usize) -> usize {
        let mut start = 0;
        for _ in 1..line.max(1) {
            match text[start..].find('\n') {
                Some(newline) => start += newline + 1,
                None => return text.len(),
            }
        }
        text[start..]
            .char_indices()
            .nth(column.saturating_sub(1))
            .map_or(text.len(), |(index, _)| start + index)
    }

    /// Index the lines of `text`, which must already have parsed as a JSON-with-comments list.
    pub fn scan(text: &str) -> Self {
        Scanner::new(text).run()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Array,
    Object,
}

struct Frame {
    kind: Kind,
    /// An object frame: the next string is a key.
    expect_key: bool,
    /// An object frame: its last key.
    key: Option<String>,
    /// An object frame that is a section's `bindings`.
    is_bindings: bool,
}

struct Scanner<'a> {
    bytes: &'a [u8],
    text: &'a str,
    pos: usize,
    line: usize,
    /// Byte offset where the current line starts, for columns.
    line_start: usize,
    stack: Vec<Frame>,
    out: SourceLines,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            text,
            pos: 0,
            line: 1,
            line_start: 0,
            stack: Vec::new(),
            out: SourceLines::default(),
        }
    }

    fn run(mut self) -> SourceLines {
        while let Some(&byte) = self.bytes.get(self.pos) {
            match byte {
                b'\n' => {
                    self.line += 1;
                    self.pos += 1;
                    self.line_start = self.pos;
                }
                b'/' if self.bytes.get(self.pos + 1) == Some(&b'/') => self.skip_line_comment(),
                b'/' if self.bytes.get(self.pos + 1) == Some(&b'*') => self.skip_block_comment(),
                b'"' => self.string(),
                b'{' => self.open(Kind::Object),
                b'[' => self.open(Kind::Array),
                b'}' | b']' => {
                    self.stack.pop();
                    self.pos += 1;
                }
                b',' => {
                    if let Some(frame) = self.stack.last_mut()
                        && frame.kind == Kind::Object
                    {
                        frame.expect_key = true;
                    }
                    self.pos += 1;
                }
                b':' | b' ' | b'\t' | b'\r' => self.pos += 1,
                _ => self.bare_value(),
            }
        }
        self.out
    }

    fn skip_line_comment(&mut self) {
        while self.bytes.get(self.pos).is_some_and(|&b| b != b'\n') {
            self.pos += 1;
        }
    }

    fn skip_block_comment(&mut self) {
        self.pos += 2;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'*' if self.bytes.get(self.pos + 1) == Some(&b'/') => {
                    self.pos += 2;
                    return;
                }
                b'\n' => {
                    self.line += 1;
                    self.line_start = self.pos + 1;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    /// The location of the token that starts at `start` (on the current line) and ends at `end`.
    fn location(&self, start: usize, end: usize) -> Location {
        let column = self
            .text
            .get(self.line_start..start)
            .map_or(1, |before| before.chars().count() + 1);
        Location {
            line: self.line,
            column,
            span: start..end,
        }
    }

    /// A value starts on the current line at `start` (a string, number, literal, object or array).
    fn value_starts(&mut self, start: usize) {
        if self.stack.len() == 1 && self.stack[0].kind == Kind::Array {
            let start = self.location(start, start + 1);
            self.out.sections.push(SectionLines {
                start,
                ..SectionLines::default()
            });
        }
    }

    fn bare_value(&mut self) {
        self.value_starts(self.pos);
        // Always moves on, so a stray `/` cannot stall the scan.
        self.pos += 1;
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| !matches!(b, b',' | b'}' | b']' | b'\n' | b' ' | b'\t' | b'\r' | b'/'))
        {
            self.pos += 1;
        }
    }

    fn open(&mut self, kind: Kind) {
        self.value_starts(self.pos);
        let is_bindings = kind == Kind::Object
            && self.stack.len() == 2
            && self.stack[1].kind == Kind::Object
            && self.stack[1].key.as_deref() == Some("bindings");
        self.stack.push(Frame {
            kind,
            expect_key: kind == Kind::Object,
            key: None,
            is_bindings,
        });
        self.pos += 1;
    }

    fn string(&mut self) {
        let start = self.pos;
        self.pos += 1;
        while let Some(&byte) = self.bytes.get(self.pos) {
            self.pos += 1;
            match byte {
                b'\\' => self.pos += 1,
                b'"' => break,
                _ => {}
            }
        }
        let is_key = self
            .stack
            .last()
            .is_some_and(|frame| frame.kind == Kind::Object && frame.expect_key);
        if !is_key {
            self.value_starts(start);
            self.context_value(start);
            return;
        }
        let literal = self.text.get(start..self.pos).unwrap_or("\"\"");
        let key: String = serde_json::from_str(literal).unwrap_or_default();
        let at = self.location(start, self.pos);
        self.key(key, at);
    }

    /// A string value at `start`: if it is the section's `context`, that is where to point.
    fn context_value(&mut self, start: usize) {
        let is_context = self.stack.len() == 2
            && self.stack[1].kind == Kind::Object
            && self.stack[1].key.as_deref() == Some("context");
        if is_context {
            let at = self.location(start, self.pos);
            if let Some(section) = self.out.sections.last_mut() {
                section.context = Some(at);
            }
        }
    }

    fn key(&mut self, key: String, at: Location) {
        let depth = self.stack.len();
        let is_bindings = self.stack.last().is_some_and(|frame| frame.is_bindings);
        if let Some(frame) = self.stack.last_mut() {
            frame.expect_key = false;
            frame.key = Some(key.clone());
        }
        let index = self.out.sections.len().wrapping_sub(1);
        let Some(section) = self.out.sections.last_mut() else {
            return;
        };
        if depth == 2 && key == "context" {
            section.context = Some(at);
        } else if depth == 3
            && is_bindings
            && let Some(first) = section.bindings.insert(key.clone(), at.clone())
        {
            self.out.duplicates.push(DuplicateKey {
                section: index,
                keystrokes: key,
                first,
                second: at,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sections_contexts_and_bindings_through_comments_and_nesting() {
        let text = r#"// header
[
  {
    "context": "Workspace", // line 4
    /* a block
       comment */
    "bindings": {
      "cmd-a": "a::One",
      "cmd-b": ["b::Two", {"n": [1, 2]}],
      "cmd-c": null,
    },
  },
  7,
  {
    "bindings": { "x": "y::Z",
      "esc\"aped": "q::R" }
  }
]"#;
        let lines = SourceLines::scan(text);
        assert_eq!(lines.section(0), Some(3));
        assert_eq!(lines.context(0), Some(4));
        assert_eq!(lines.binding(0, "cmd-a"), Some(8));
        assert_eq!(lines.binding(0, "cmd-b"), Some(9));
        assert_eq!(lines.binding(0, "cmd-c"), Some(10));
        assert_eq!(
            lines.section(1),
            Some(13),
            "a non-object section still counts"
        );
        assert_eq!(lines.section(2), Some(14));
        assert_eq!(lines.context(2), None);
        assert_eq!(lines.binding(2, "x"), Some(15));
        assert_eq!(lines.binding(2, "esc\"aped"), Some(16));
        assert_eq!(lines.section(3), None);
        assert_eq!(lines.binding(0, "missing"), None);
    }

    #[test]
    fn a_key_with_nested_keys_of_the_same_name_is_not_confused() {
        let lines = SourceLines::scan(
            "[{\n\"bindings\": {\n\"a\": [\"x::Y\", {\"context\": 1, \"bindings\": {}}]\n}}]",
        );
        assert_eq!(lines.context(0), None);
        assert_eq!(lines.binding(0, "a"), Some(3));
    }

    #[test]
    fn a_duplicate_key_keeps_the_later_line() {
        let lines = SourceLines::scan("[{\"bindings\": {\n\"a\": \"x::Y\",\n\"a\": \"x::Z\"}}]");
        assert_eq!(lines.binding(0, "a"), Some(3));
    }

    #[test]
    fn locations_carry_columns_and_byte_spans() {
        let text = "[\n  {\"context\": \"Pane\",\n    \"bindings\": {\"cmd-a\": \"é::One\", \"ctrl-b\": null}}\n]";
        let lines = SourceLines::scan(text);
        let section = lines.section_location(0).unwrap();
        assert_eq!((section.line, section.column), (2, 3));
        let context = lines.context_location(0).unwrap();
        assert_eq!(&text[context.span.clone()], "\"Pane\"");
        assert_eq!((context.line, context.column), (2, 15));
        let a = lines.binding_location(0, "cmd-a").unwrap();
        assert_eq!(&text[a.span.clone()], "\"cmd-a\"");
        assert_eq!((a.line, a.column), (3, 18));
        let b = lines.binding_location(0, "ctrl-b").unwrap();
        assert_eq!(&text[b.span.clone()], "\"ctrl-b\"");
        // The column counts characters, not bytes, after the multi-byte `é`.
        assert_eq!(b.column, 37);
    }

    #[test]
    fn a_repeated_binding_key_is_listed_with_both_places() {
        let lines = SourceLines::scan("[{\"bindings\": {\n\"a\": \"x::Y\",\n\"a\": \"x::Z\"}}]");
        let [duplicate] = lines.duplicates() else {
            panic!("one duplicate");
        };
        assert_eq!(duplicate.keystrokes, "a");
        assert_eq!((duplicate.first.line, duplicate.second.line), (2, 3));
    }

    #[test]
    fn position_and_offset_are_inverse_and_clamped() {
        let text = "ab\ncé\nd";
        for offset in [0, 1, 3, 4, 7] {
            let (line, column) = SourceLines::position(text, offset);
            assert_eq!(SourceLines::offset(text, line, column), offset);
        }
        assert_eq!(SourceLines::position(text, 99), (3, 2));
        assert_eq!(SourceLines::offset(text, 9, 1), text.len());
        assert_eq!(
            SourceLines::position(text, 5),
            (2, 2),
            "inside a character rounds down"
        );
    }

    #[test]
    fn text_it_cannot_follow_just_knows_fewer_lines() {
        for text in [
            "",
            "[",
            "{",
            "\"",
            "[{\"bindings\": {\"a\": \"b",
            "/* never closed",
        ] {
            let _ = SourceLines::scan(text);
        }
    }
}
