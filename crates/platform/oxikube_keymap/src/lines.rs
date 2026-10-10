//! Where things are in a `keymap.json`: the 1-based line of each section, its `context` and each
//! of its bindings, so a diagnostic can say `keymap.json:12`.
//!
//! `serde_json` parses to values without positions, and by the time a binding is rejected the
//! text has long been parsed. This is a second, tiny pass over text that already parsed as JSON
//! with comments: it skips comments and strings, follows the nesting, and records the lines of
//! the keys it cares about. It never fails; on text it cannot follow it just knows fewer lines.

use std::collections::HashMap;

/// The lines of one section (an element of the top-level list).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SectionLines {
    start: usize,
    context: Option<usize>,
    /// Binding key (as written, unescaped) to the line it starts on. A key written twice keeps
    /// the later line, which is the binding that wins.
    bindings: HashMap<String, usize>,
}

/// The lines of the sections of one keymap file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceLines {
    sections: Vec<SectionLines>,
}

impl SourceLines {
    /// The line the `index`th section starts on.
    pub fn section(&self, index: usize) -> Option<usize> {
        self.sections.get(index).map(|section| section.start)
    }

    /// The line of the `context` key of the `index`th section.
    pub fn context(&self, index: usize) -> Option<usize> {
        self.sections.get(index)?.context
    }

    /// The line of the binding `keystrokes` in the `index`th section.
    pub fn binding(&self, index: usize, keystrokes: &str) -> Option<usize> {
        self.sections.get(index)?.bindings.get(keystrokes).copied()
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
                b'\n' => self.line += 1,
                _ => {}
            }
            self.pos += 1;
        }
    }

    /// A value starts on the current line (a string, number, literal, object or array).
    fn value_starts(&mut self) {
        if self.stack.len() == 1 && self.stack[0].kind == Kind::Array {
            self.out.sections.push(SectionLines {
                start: self.line,
                ..SectionLines::default()
            });
        }
    }

    fn bare_value(&mut self) {
        self.value_starts();
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
        self.value_starts();
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
            self.value_starts();
            return;
        }
        let literal = self.text.get(start..self.pos).unwrap_or("\"\"");
        let key: String = serde_json::from_str(literal).unwrap_or_default();
        self.key(key);
    }

    fn key(&mut self, key: String) {
        let depth = self.stack.len();
        let line = self.line;
        let is_bindings = self.stack.last().is_some_and(|frame| frame.is_bindings);
        if let Some(frame) = self.stack.last_mut() {
            frame.expect_key = false;
            frame.key = Some(key.clone());
        }
        let Some(section) = self.out.sections.last_mut() else {
            return;
        };
        if depth == 2 && key == "context" {
            section.context = Some(line);
        } else if depth == 3 && is_bindings {
            section.bindings.insert(key, line);
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
