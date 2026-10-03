//! The scrubber: byte-level pre-check, then only the patterns that could match.

use super::patterns::{
    AUTHORIZATION_SRC, BEARER_SRC, DATA_FLOW_SRC, DATA_HEADER_SRC, DATA_PAIR_SRC, JWT_SRC, MARKER,
    PEM_SRC, SECRET_FIELD_SRC, URL_USERINFO_SRC,
};
use regex::{Captures, Regex};
use std::borrow::Cow;
use std::sync::LazyLock;

/// Compiles on first use. The sources are literals covered by tests, so a failure is a
/// programming error caught in CI, never a runtime condition.
macro_rules! lazy_regex {
    ($name:ident, $src:expr) => {
        static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($src).expect("redaction pattern is a tested literal"));
    };
}

lazy_regex!(PEM, PEM_SRC);
lazy_regex!(AUTHORIZATION, AUTHORIZATION_SRC);
lazy_regex!(SECRET_FIELD, SECRET_FIELD_SRC);
lazy_regex!(URL_USERINFO, URL_USERINFO_SRC);
lazy_regex!(BEARER, BEARER_SRC);
lazy_regex!(JWT, JWT_SRC);
lazy_regex!(DATA_FLOW, DATA_FLOW_SRC);
lazy_regex!(DATA_PAIR, DATA_PAIR_SRC);
lazy_regex!(DATA_HEADER, DATA_HEADER_SRC);

// Which stages the pre-check says could match.
const F_PEM: u8 = 1;
const F_AUTH: u8 = 1 << 1;
const F_FIELD: u8 = 1 << 2;
const F_BEARER: u8 = 1 << 3;
const F_JWT: u8 = 1 << 4;
const F_DATA: u8 = 1 << 5;
const F_URL: u8 = 1 << 6;
const F_ALL: u8 = F_PEM | F_AUTH | F_FIELD | F_BEARER | F_JWT | F_DATA | F_URL;

/// Replaces secret-bearing substrings of `input` with [`MARKER`].
///
/// Returns `Cow::Borrowed(input)` (no allocation, no regex) when the byte-level pre-check finds
/// no candidate. Idempotent. See the [module docs](super) for the pattern list.
pub fn redact(input: &str) -> Cow<'_, str> {
    let flags = candidates(input.as_bytes());
    if flags == 0 {
        return Cow::Borrowed(input);
    }
    let mut out = Cow::Borrowed(input);
    if flags & F_PEM != 0 {
        out = step(out, |s| PEM.replace_all(s, MARKER));
    }
    if flags & F_URL != 0 {
        out = step(out, |s| URL_USERINFO.replace_all(s, replace_url_userinfo));
    }
    if flags & F_AUTH != 0 {
        out = step(out, |s| AUTHORIZATION.replace_all(s, replace_assignment));
    }
    if flags & F_FIELD != 0 {
        out = step(out, |s| SECRET_FIELD.replace_all(s, replace_assignment));
    }
    if flags & F_BEARER != 0 {
        out = step(out, |s| BEARER.replace_all(s, replace_bearer));
    }
    if flags & F_JWT != 0 {
        out = step(out, |s| JWT.replace_all(s, MARKER));
    }
    if flags & F_DATA != 0 {
        out = step(out, |s| DATA_FLOW.replace_all(s, replace_data_flow));
        out = step(out, scrub_data_blocks);
    }
    out
}

/// Applies one stage, staying borrowed when the stage changed nothing.
fn step<'a>(current: Cow<'a, str>, stage: impl FnOnce(&str) -> Cow<'_, str>) -> Cow<'a, str> {
    match stage(&current) {
        Cow::Borrowed(_) => current,
        Cow::Owned(changed) => Cow::Owned(changed),
    }
}

/// Single pass over the bytes: which stages could match?
fn candidates(s: &[u8]) -> u8 {
    let mut flags = 0;
    let (mut scheme_sep, mut at_sign) = (false, false);
    for i in 0..s.len() {
        let rest = &s[i..];
        match rest[0].to_ascii_lowercase() {
            b'a' if starts_ci(rest, b"authorization") => flags |= F_AUTH,
            b'b' if starts_ci(rest, b"bearer") => flags |= F_BEARER,
            b't' if starts_ci(rest, b"token") => flags |= F_FIELD,
            b'p' if starts_ci(rest, b"password") || starts_ci(rest, b"passwd") => flags |= F_FIELD,
            b'k' if starts_ci(rest, b"key-data") || starts_ci(rest, b"key_data") => {
                flags |= F_FIELD;
            }
            b'c' if starts_ci(rest, b"certificate-data")
                || starts_ci(rest, b"certificate_data")
                || starts_ci(rest, b"client-secret")
                || starts_ci(rest, b"client_secret") =>
            {
                flags |= F_FIELD;
            }
            b'-' if starts_ci(rest, b"-----begin") => flags |= F_PEM,
            b'e' if rest.starts_with(b"eyJ") => flags |= F_JWT,
            b'd' if data_key(rest) => flags |= F_DATA,
            b':' if rest.starts_with(b"://") => scheme_sep = true,
            b'@' => at_sign = true,
            _ => {}
        }
        if scheme_sep && at_sign {
            flags |= F_URL;
        }
        if flags == F_ALL {
            break;
        }
    }
    flags
}

fn starts_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len() && hay[..needle.len()].eq_ignore_ascii_case(needle)
}

/// `data` (any case, so `stringData` too) followed by an optional closing quote and a `:` or `=`.
fn data_key(rest: &[u8]) -> bool {
    starts_ci(rest, b"data")
        && rest[4..]
            .iter()
            .find(|b| !matches!(b, b'"' | b'\'' | b'\\' | b' ' | b'\t'))
            .is_some_and(|b| matches!(b, b':' | b'='))
}

/// The replacement for `val`, keeping its quote style, or `None` when it is already redacted.
fn replacement(val: &str) -> Option<&'static str> {
    if val.trim_matches(['"', '\'', '\\']) == MARKER {
        None
    } else if val.starts_with("\\\"") {
        Some("\\\"[redacted]\\\"")
    } else if val.starts_with('"') {
        Some("\"[redacted]\"")
    } else if val.starts_with('\'') {
        Some("'[redacted]'")
    } else {
        Some(MARKER)
    }
}

/// `key`, `sep`, `val` groups: keep key and separator, replace the value.
fn replace_assignment(caps: &Captures<'_>) -> String {
    let val = &caps["val"];
    match replacement(val) {
        None => caps[0].to_owned(),
        Some(rep) => format!("{}{}{rep}", &caps["key"], &caps["sep"]),
    }
}

/// `scheme://user:password@host`: keep everything but the password.
fn replace_url_userinfo(caps: &Captures<'_>) -> String {
    format!("{}{MARKER}@", &caps["pre"])
}

/// `Bearer <token>`: keep the scheme word, unless the "token" is a plain word.
fn replace_bearer(caps: &Captures<'_>) -> String {
    let tok = &caps["tok"];
    let plain_word = tok.len() < 20 && tok.bytes().all(|b| b.is_ascii_alphabetic());
    if plain_word {
        return caps[0].to_owned();
    }
    let (Some(whole), Some(tok_match)) = (caps.get(0), caps.name("tok")) else {
        return caps[0].to_owned();
    };
    let scheme = &whole.as_str()[..tok_match.start() - whole.start()];
    format!("{scheme}{MARKER}")
}

/// An inline `data` map: redact every entry's value.
fn replace_data_flow(caps: &Captures<'_>) -> String {
    let body = DATA_PAIR.replace_all(&caps["body"], replace_pair);
    format!("{}{}{}{body}}}", &caps["key"], &caps["sep"], &caps["open"])
}

fn replace_pair(caps: &Captures<'_>) -> String {
    match replacement(&caps["val"]) {
        None => caps[0].to_owned(),
        Some(rep) => format!("{}{}{rep}", &caps["k"], &caps["s"]),
    }
}

/// Multi-line `data:` / `stringData:` blocks: redact the entries indented under the header.
fn scrub_data_blocks(input: &str) -> Cow<'_, str> {
    if !input.lines().any(|l| DATA_HEADER.is_match(l)) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut block_indent: Option<usize> = None;
    for line in input.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let indent = content.len() - content.trim_start().len();
        if let Some(header_indent) = block_indent {
            if content.trim().is_empty() {
                out.push_str(line);
                continue;
            }
            if indent > header_indent {
                out.push_str(&DATA_PAIR.replace_all(line, replace_pair));
                continue;
            }
            block_indent = None;
        }
        if DATA_HEADER.is_match(content) {
            block_indent = Some(indent);
        }
        out.push_str(line);
    }
    if out == input {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(out)
    }
}
