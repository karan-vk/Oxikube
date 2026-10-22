//! [`sanitize`]: makes untrusted Markdown safe to show.

use super::links::{Pairs, badge_or_image};

/// The longest source [`sanitize`] keeps; the rest is cut at a line break.
pub const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;

/// Appended where [`sanitize`] cut a document.
pub const TRUNCATED_NOTE: &str = "\n\n*The rest of this document is not shown (too large).*\n";

/// Whether a clicked link may be opened in the system browser: `http`, `https` and `mailto` only.
/// Relative links, `javascript:`, `file:` and everything else are refused.
pub fn is_safe_link(url: &str) -> bool {
    let url = url.trim();
    let scheme = url.split(':').next().unwrap_or_default();
    url.contains(':')
        && ["http", "https", "mailto"]
            .iter()
            .any(|s| scheme.eq_ignore_ascii_case(s))
}

/// Strips what must not be rendered from `source`:
///
/// * raw HTML: tags are removed (their text stays), comments and `<script>` / `<style>` elements
///   are removed with their content; `<https://autolinks>` and code (fenced blocks, inline code)
///   are left alone;
/// * images: `![alt](url)` becomes the link `[alt](url)` (`[image](url)` without alt text), and a
///   linked badge `[![alt](img)](target)` becomes `[alt](target)`, so showing a document never
///   fetches a remote image;
/// * size: a source over [`MAX_SOURCE_BYTES`] is cut at a line break and [`TRUNCATED_NOTE`] added.
///
/// Pure and linear in the input (every scan of a line is bounded by what it consumes, so an
/// untrusted document of unclosed `![` or repeated `<script></script>` costs no more than plain
/// text), but run it off the UI thread for big documents.
pub fn sanitize(source: &str) -> String {
    let (source, cut) = if source.len() > MAX_SOURCE_BYTES {
        let mut end = MAX_SOURCE_BYTES;
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        let end = source[..end].rfind('\n').unwrap_or(end);
        (&source[..end], true)
    } else {
        (source, false)
    };
    let mut out = String::with_capacity(source.len());
    let mut state = State::default();
    for line in source.split_inclusive('\n') {
        state.line(line, &mut out);
    }
    if cut {
        out.push_str(TRUNCATED_NOTE);
    }
    out
}

#[derive(Default)]
struct State {
    /// Inside a fenced code block: the fence character and its length.
    fence: Option<(char, usize)>,
    /// Inside a `<!-- -->` comment.
    comment: bool,
    /// Inside `<script>` or `<style>`: the closing tag to wait for.
    raw_until: Option<&'static str>,
    /// Inside a tag whose `>` has not come yet.
    in_tag: bool,
}

impl State {
    fn line(&mut self, line: &str, out: &mut String) {
        if let Some((ch, len)) = self.fence {
            if closes_fence(line, ch, len) {
                self.fence = None;
            }
            out.push_str(line);
            return;
        }
        if self.comment || self.raw_until.is_some() || self.in_tag {
            let rest = self.skip_pending(line);
            if self.comment || self.raw_until.is_some() || self.in_tag {
                // Still inside; keep the line break so the text around keeps its blocks.
                if line.ends_with('\n') {
                    out.push('\n');
                }
                return;
            }
            self.inline(rest, out);
            return;
        }
        if let Some(open) = opens_fence(line) {
            self.fence = Some(open);
            out.push_str(line);
            return;
        }
        self.inline(line, out);
    }

    /// Consumes the part of `line` that closes the pending comment, raw element or tag; returns
    /// what follows it.
    fn skip_pending<'a>(&mut self, line: &'a str) -> &'a str {
        if self.comment {
            match line.find("-->") {
                Some(at) => {
                    self.comment = false;
                    return &line[at + 3..];
                }
                None => return "",
            }
        }
        if let Some(close) = self.raw_until {
            let lower = line.to_ascii_lowercase();
            match lower.find(close) {
                Some(at) => {
                    self.raw_until = None;
                    let rest = &line[at + close.len()..];
                    // Finish the closing tag itself.
                    return match rest.find('>') {
                        Some(gt) => &rest[gt + 1..],
                        None => {
                            self.in_tag = true;
                            ""
                        }
                    };
                }
                None => return "",
            }
        }
        match line.find('>') {
            Some(at) => {
                self.in_tag = false;
                &line[at + 1..]
            }
            None => "",
        }
    }

    fn inline(&mut self, line: &str, out: &mut String) {
        let chars: Vec<char> = line.chars().collect();
        // Built the first time an image or badge may start: most lines have neither.
        let mut pairs: Option<Pairs> = None;
        let mut i = 0;
        let mut code: Option<usize> = None;
        while i < chars.len() {
            let c = chars[i];
            if c == '`' {
                let run = chars[i..].iter().take_while(|&&c| c == '`').count();
                code = match code {
                    None => Some(run),
                    Some(open) if open == run => None,
                    other => other,
                };
                out.extend(&chars[i..i + run]);
                i += run;
                continue;
            }
            if code.is_some() {
                out.push(c);
                i += 1;
                continue;
            }
            if c == '<'
                && let Some(consumed) = self.html(&chars[i..])
            {
                i += consumed;
                // The construct runs on into the next line: keep this line's break.
                if (self.comment || self.raw_until.is_some() || self.in_tag)
                    && chars.last() == Some(&'\n')
                {
                    out.push('\n');
                }
                continue;
            }
            if (c == '[' || c == '!') && chars.get(i + 1).is_some_and(|n| matches!(n, '[' | '!')) {
                let pairs = pairs.get_or_insert_with(|| Pairs::of(&chars));
                if let Some((text, consumed)) = badge_or_image(&chars, i, pairs) {
                    out.push_str(&text);
                    i += consumed;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
    }

    /// If `rest` starts an HTML construct, consumes it (setting the pending state when it runs
    /// past the line) and returns how many chars; `None` when the `<` is plain text or an
    /// autolink.
    fn html(&mut self, rest: &[char]) -> Option<usize> {
        let starts = |s: &str| {
            rest.len() >= s.len()
                && rest[..s.len()]
                    .iter()
                    .zip(s.chars())
                    .all(|(a, b)| a.eq_ignore_ascii_case(&b))
        };
        if starts("<!--") {
            return Some(match find(rest, "-->", 4) {
                Some(end) => end + 3,
                None => {
                    self.comment = true;
                    rest.len()
                }
            });
        }
        let after_slash = usize::from(rest.get(1) == Some(&'/')) + 1;
        let name_len = rest[after_slash.min(rest.len())..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == '-')
            .count();
        let named = rest.get(after_slash).is_some_and(char::is_ascii_alphabetic);
        let boundary = rest
            .get(after_slash + name_len)
            .is_none_or(|c| c.is_whitespace() || *c == '/' || *c == '>');
        // `<!DOCTYPE ...>` and `<?xml ...?>` are HTML noise too.
        let declaration = matches!(rest.get(1), Some('!' | '?'));
        if !(named && boundary) && !declaration {
            return None;
        }
        let name: String = rest[after_slash..after_slash + name_len]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        let opening = after_slash == 1;
        let end = match rest.iter().position(|&c| c == '>') {
            Some(gt) => gt + 1,
            None => {
                self.in_tag = true;
                rest.len()
            }
        };
        if opening
            && !self.in_tag
            && (name == "script" || name == "style")
            && !rest[..end - 1].ends_with(&['/'])
        {
            // Drop the element's content up to its closing tag (on this line or later ones).
            let close: &'static str = if name == "script" {
                "</script"
            } else {
                "</style"
            };
            // Scans only up to the closing tag (never copies the rest of the line), so many
            // elements on one line stay linear.
            let tail = &rest[end..];
            return Some(match find_ignore_case(tail, close) {
                Some(at) => {
                    let after = at + close.len();
                    let gt = tail[after..]
                        .iter()
                        .position(|&c| c == '>')
                        .map_or(tail.len() - after, |g| g + 1);
                    end + after + gt
                }
                None => {
                    self.raw_until = Some(close);
                    rest.len()
                }
            });
        }
        Some(end)
    }
}

/// The first index of ASCII `needle` in `hay`, ignoring ASCII case.
fn find_ignore_case(hay: &[char], needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    (0..hay.len().saturating_sub(needle.len() - 1)).find(|&i| {
        hay[i..i + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

fn find(hay: &[char], needle: &str, from: usize) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    (from..hay.len().saturating_sub(needle.len() - 1))
        .find(|&i| hay[i..i + needle.len()] == needle[..])
}

fn opens_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let ch = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = trimmed.chars().take_while(|&c| c == ch).count();
    // A backtick fence's info string cannot contain a backtick (that is inline code).
    (len >= 3 && !(ch == '`' && trimmed[len..].contains('`'))).then_some((ch, len))
}

fn closes_fence(line: &str, ch: char, len: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    let run = trimmed.chars().take_while(|&c| c == ch).count();
    run >= len && trimmed[run..].trim().is_empty()
}
