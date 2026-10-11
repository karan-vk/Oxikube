//! The strict JSON check of a buffer in JSON mode.
//!
//! JSON is a subset of YAML 1.2 flow style, so the spanned YAML model (E10-S02) parses a JSON
//! buffer and the schema validator (E10-S03) runs on it unchanged. What the YAML parser also
//! accepts is not JSON: a trailing comma, `'single quotes'`, bare words and comments. This
//! scanner finds the first such place, with the span to underline, so the editor reports it as
//! a `syntax` diagnostic. It checks the grammar of RFC 8259 and nothing else: no value is built.

use std::ops::Range;

/// How deep arrays and objects may nest before the scanner gives up (a stack guard).
const MAX_DEPTH: usize = 256;

/// The first place a text stops being JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// Bytes to underline: always on character boundaries, never empty unless the text ended.
    pub span: Range<usize>,
    /// What is wrong, in one line.
    pub message: String,
}

/// The first syntax error of `text` read as one JSON value, `None` when it is JSON (or empty or
/// blank: an empty buffer is not an error). Linear time and constant extra memory besides the
/// nesting depth.
#[must_use]
pub fn first_error(text: &str) -> Option<JsonError> {
    let mut scanner = Scanner {
        text,
        bytes: text.as_bytes(),
        at: 0,
    };
    scanner.skip_blank();
    if scanner.at >= scanner.bytes.len() {
        return None;
    }
    scanner.value(0).err().or_else(|| {
        scanner.skip_blank();
        (scanner.at < scanner.bytes.len())
            .then(|| scanner.error_here("unexpected text after the JSON value"))
    })
}

struct Scanner<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
}

type Step = Result<(), JsonError>;

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_blank(&mut self) {
        // A byte-order mark may open a file.
        if self.at == 0 && self.text.starts_with('\u{feff}') {
            self.at = '\u{feff}'.len_utf8();
        }
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// An error on the character at the cursor (or at the end of the text).
    fn error_here(&self, message: impl Into<String>) -> JsonError {
        let start = self.at.min(self.text.len());
        let width = self.text[start..].chars().next().map_or(0, char::len_utf8);
        self.error_at(start..start + width, message)
    }

    fn error_at(&self, span: Range<usize>, message: impl Into<String>) -> JsonError {
        JsonError {
            span,
            message: message.into(),
        }
    }

    fn value(&mut self, depth: usize) -> Step {
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b't') => self.word("true"),
            Some(b'f') => self.word("false"),
            Some(b'n') => self.word("null"),
            Some(b'\'') => Err(self.error_here("JSON strings use double quotes")),
            Some(b'#') => Err(self.error_here("JSON has no comments")),
            Some(b'/') if matches!(self.bytes.get(self.at + 1), Some(b'/' | b'*')) => {
                Err(self.error_here("JSON has no comments"))
            }
            Some(b',' | b']' | b'}' | b':') => Err(self.error_here("a value is expected here")),
            Some(b) if b.is_ascii_alphabetic() => Err(self.bare_word("a value is expected")),
            Some(_) => Err(self.error_here("unexpected character, a JSON value is expected")),
            None => Err(self.error_here("the JSON ends where a value is expected")),
        }
    }

    fn word(&mut self, word: &str) -> Step {
        if self.text[self.at..].starts_with(word) {
            self.at += word.len();
            if matches!(self.peek(), Some(b) if b.is_ascii_alphanumeric() || b == b'_') {
                return Err(self.error_here(format!("expected `{word}`")));
            }
            Ok(())
        } else {
            Err(self.bare_word(&format!("did you mean `{word}`?")))
        }
    }

    /// An error spanning the run of letters, digits and `_` at the cursor.
    fn bare_word(&self, hint: &str) -> JsonError {
        let end = self.text[self.at..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map_or(self.text.len(), |n| self.at + n);
        self.error_at(
            self.at..end.max(self.at + 1),
            format!("a bare word is not JSON ({hint})"),
        )
    }

    fn nested(&self, depth: usize) -> Step {
        if depth >= MAX_DEPTH {
            Err(self.error_here("nested too deeply"))
        } else {
            Ok(())
        }
    }

    fn object(&mut self, depth: usize) -> Step {
        self.nested(depth)?;
        self.at += 1;
        self.skip_blank();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(());
        }
        loop {
            if self.peek() != Some(b'"') {
                return Err(match self.peek() {
                    Some(b'\'') => self.error_here("JSON strings use double quotes"),
                    Some(_) => self.error_here("an object key must be a string in double quotes"),
                    None => self.error_here("the object is not closed"),
                });
            }
            self.string()?;
            self.skip_blank();
            if self.peek() != Some(b':') {
                return Err(self.error_here("`:` is expected after the key"));
            }
            self.at += 1;
            self.skip_blank();
            self.value(depth + 1)?;
            if self.separator(b'}', "the object")? {
                return Ok(());
            }
        }
    }

    fn array(&mut self, depth: usize) -> Step {
        self.nested(depth)?;
        self.at += 1;
        self.skip_blank();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(());
        }
        loop {
            self.value(depth + 1)?;
            if self.separator(b']', "the array")? {
                return Ok(());
            }
        }
    }

    /// After an element: `,` (then the next element must follow: a closer here is a trailing
    /// comma) or the `close` byte. `Ok(true)` once `close` was consumed.
    fn separator(&mut self, close: u8, what: &str) -> Result<bool, JsonError> {
        self.skip_blank();
        match self.peek() {
            Some(b',') => {
                let comma = self.at;
                self.at += 1;
                self.skip_blank();
                if self.peek() == Some(close) {
                    return Err(
                        self.error_at(comma..comma + 1, "trailing comma is not allowed in JSON")
                    );
                }
                Ok(false)
            }
            Some(b) if b == close => {
                self.at += 1;
                Ok(true)
            }
            Some(_) => Err(self.error_here(format!(
                "`,` or `{}` is expected after the value",
                close as char
            ))),
            None => Err(self.error_here(format!("{what} is not closed"))),
        }
    }

    fn string(&mut self) -> Step {
        let open = self.at;
        self.at += 1;
        loop {
            match self.peek() {
                None => {
                    return Err(self.error_at(open..open + 1, "the string is not closed"));
                }
                Some(b'"') => {
                    self.at += 1;
                    return Ok(());
                }
                Some(b'\\') => self.escape()?,
                Some(b) if b < 0x20 => {
                    return Err(self.error_here(if b == b'\n' || b == b'\r' {
                        "a string cannot span lines (use \\n)"
                    } else {
                        "a control character must be escaped"
                    }));
                }
                Some(_) => self.at += 1,
            }
        }
    }

    fn escape(&mut self) -> Step {
        let start = self.at;
        self.at += 1;
        match self.peek() {
            Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                self.at += 1;
                Ok(())
            }
            Some(b'u') => {
                self.at += 1;
                for _ in 0..4 {
                    if !matches!(self.peek(), Some(b) if b.is_ascii_hexdigit()) {
                        return Err(self
                            .error_at(start..self.at.max(start + 2), "\\u needs four hex digits"));
                    }
                    self.at += 1;
                }
                Ok(())
            }
            _ => {
                let end = self.at
                    + self.text[self.at..]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8);
                Err(self.error_at(start..end, "invalid escape sequence"))
            }
        }
    }

    fn number(&mut self) -> Step {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.number_error(start, "a number cannot have leading zeros"));
                }
            }
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.number_error(start, "invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.number_error(start, "digits are expected after the decimal point"));
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.number_error(start, "digits are expected in the exponent"));
            }
            self.digits();
        }
        Ok(())
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
    }

    /// An error spanning the number from `start` to the end of the run of number characters.
    fn number_error(&self, start: usize, message: &str) -> JsonError {
        let mut end = self.at.max(start + 1);
        while matches!(self.bytes.get(end), Some(b) if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
        {
            end += 1;
        }
        self.error_at(start..end, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(text: &str) -> JsonError {
        first_error(text).unwrap_or_else(|| panic!("{text:?} should not be JSON"))
    }

    fn slice<'a>(text: &'a str, e: &JsonError) -> &'a str {
        &text[e.span.clone()]
    }

    #[test]
    fn valid_json_has_no_error() {
        for text in [
            "",
            "  \n",
            "{}",
            "[]",
            "null",
            "-0.5e+3",
            "{\"a\": [1, 2.5, true, false, null, \"x\\n\\u00e9\\ud83d\\ude00\"], \"b\": {}}",
            "\u{feff}{\"kind\":\"Pod\"}",
            "{\n\t\"tab\":\t1\n}\n",
            "12345678901234567890123456789",
            "{\"é\": \"日本語 😀\"}",
        ] {
            assert_eq!(first_error(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_trailing_comma_is_underlined() {
        let text = "{\"a\": 1,\n}";
        let e = error(text);
        assert_eq!(slice(text, &e), ",");
        assert!(e.message.contains("trailing comma"));
        let text = "{\"a\": [1, 2 , ]}";
        assert_eq!(slice(text, &error(text)), ",");
    }

    #[test]
    fn yaml_only_spellings_are_not_json() {
        for (text, bad) in [
            ("{'a': 1}", "'"),
            ("{\"a\": 'x'}", "'"),
            ("{a: 1}", "a"),
            ("{\"a\": yes}", "yes"),
            ("{\"a\": nul}", "nul"),
            ("{\"a\": 1} # note", "#"),
            ("// c\n{}", "/"),
            ("{\"a\" 1}", "1"),
            ("{\"a\": 1 \"b\": 2}", "\""),
            ("{\"a\": 01}", "01"),
            ("{\"a\": .5}", "."),
            ("{\"a\": 1.}", "1."),
            ("{\"a\": +1}", "+"),
            ("{\"a\": 1e}", "1e"),
            ("{} {}", "{"),
            ("{\"a\": \"x\ty\"}", "\t"),
            ("{\"a\": \"\\q\"}", "\\q"),
            ("{\"a\": \"\\u12G4\"}", "\\u12"),
            ("[1,,2]", ","),
            ("[,]", ","),
            ("{\"a\":}", "}"),
        ] {
            let e = error(text);
            assert_eq!(slice(text, &e), bad, "{text:?}: {e:?}");
        }
    }

    #[test]
    fn unclosed_things_say_what_is_open() {
        let text = "{\"a\": [1, 2";
        let e = error(text);
        assert!(e.message.contains("array is not closed"), "{e:?}");
        let text = "{\"a\": \"x";
        let e = error(text);
        assert_eq!(slice(text, &e), "\"");
        assert!(e.message.contains("not closed"));
        assert!(error("{").message.contains("not closed"));
    }

    #[test]
    fn spans_stay_on_character_boundaries() {
        let text = "{\"a\": é}";
        let e = error(text);
        assert_eq!(slice(text, &e), "é");
        let text = "[\"x\" é]";
        assert_eq!(slice(text, &error(text)), "é");
    }

    #[test]
    fn very_deep_nesting_is_refused_not_overflowed() {
        let text = "[".repeat(10_000);
        assert!(error(&text).message.contains("deeply"));
        let ok = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert_eq!(first_error(&ok), None);
    }
}
