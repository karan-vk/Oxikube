//! Images and linked badges inside one line, found in linear time.
//!
//! [`Pairs`] matches every `[`/`]` and `(`/`)` of a line once, with a stack, so looking up where a
//! bracket closes is O(1). Without it each `![` or `[!` scanned to the end of its line, and a line
//! of a few hundred KB of unclosed brackets took seconds to minutes (an untrusted README can be
//! exactly that).

/// No partner.
const NONE: u32 = u32::MAX;

/// The closing partner of every opening bracket and parenthesis of a line.
pub(super) struct Pairs {
    /// For a `[` at `i`, the index of its `]`; [`NONE`] otherwise.
    bracket: Vec<u32>,
    /// For a `(` at `i`, the index of its `)`; [`NONE`] otherwise.
    paren: Vec<u32>,
}

impl Pairs {
    /// Matches the brackets and parentheses of `chars` (one line). Backslashes do not escape.
    pub(super) fn of(chars: &[char]) -> Self {
        let mut bracket = vec![NONE; chars.len()];
        let mut paren = vec![NONE; chars.len()];
        let (mut squares, mut rounds) = (Vec::new(), Vec::new());
        for (i, &c) in chars.iter().enumerate() {
            match c {
                '[' => squares.push(i),
                ']' => {
                    if let Some(open) = squares.pop() {
                        bracket[open] = i as u32;
                    }
                }
                '(' => rounds.push(i),
                ')' => {
                    if let Some(open) = rounds.pop() {
                        paren[open] = i as u32;
                    }
                }
                _ => {}
            }
        }
        Self { bracket, paren }
    }

    fn close(table: &[u32], at: usize) -> Option<usize> {
        table
            .get(at)
            .filter(|&&close| close != NONE)
            .map(|&close| close as usize)
    }
}

/// `![alt](url)` or `[![alt](img)](target)` at index `at` of `chars`, as the link it becomes and
/// how many chars it covers.
pub(super) fn badge_or_image(chars: &[char], at: usize, pairs: &Pairs) -> Option<(String, usize)> {
    if chars.get(at) == Some(&'!') {
        let (alt, url, used) = bracket_link(chars, at + 1, pairs)?;
        return Some((link(&alt, &url), used + 1));
    }
    // `[` + image + `](target)`.
    if chars.get(at) == Some(&'[') && chars.get(at + 1) == Some(&'!') {
        let (alt, _img, used) = bracket_link(chars, at + 2, pairs)?;
        let tail = at + 2 + used;
        if chars.get(tail) == Some(&']') && chars.get(tail + 1) == Some(&'(') {
            let (target, len) = destination(chars, tail + 1, pairs)?;
            return Some((link(&alt, &target), 2 + used + 1 + len));
        }
    }
    None
}

fn link(alt: &str, url: &str) -> String {
    let alt = plain_text(alt.trim());
    format!(
        "[{}]({url})",
        if alt.is_empty() {
            "image"
        } else {
            alt.as_str()
        }
    )
}

/// `text` (an image's alt, now a link's text) without anything that could render as an image or
/// HTML: a nested `![...](...)` becomes `[...](...)` and `<`/`>` are dropped. Linear in `text`.
fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '!' if chars.peek() == Some(&'[') => {}
            '<' | '>' => {}
            c => out.push(c),
        }
    }
    out
}

/// `[text](dest ...)` at index `at`: the text, the destination and the chars used.
fn bracket_link(chars: &[char], at: usize, pairs: &Pairs) -> Option<(String, String, usize)> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let close = Pairs::close(&pairs.bracket, at)?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let (url, len) = destination(chars, close + 1, pairs)?;
    let alt: String = chars[at + 1..close].iter().collect();
    Some((alt, url, close - at + 1 + len))
}

/// `(url "title")` at index `at`: the url and the chars used (both parens included).
fn destination(chars: &[char], at: usize, pairs: &Pairs) -> Option<(String, usize)> {
    if chars.get(at) != Some(&'(') {
        return None;
    }
    let close = Pairs::close(&pairs.paren, at)?;
    let inner: String = chars[at + 1..close].iter().collect();
    // Drop an optional title.
    let url = inner
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    Some((url, close - at + 1))
}
