//! Go-style durations (`14.982s`, `1m30s`, `250ms`), the form `metrics.k8s.io` uses for `window`.

use jiff::SignedDuration;

/// Nanoseconds per unit, `ms` is listed before `m` so the longer spelling wins.
const UNITS: [(&str, f64); 8] = [
    ("ns", 1.0),
    ("us", 1_000.0),
    ("\u{b5}s", 1_000.0),
    ("\u{3bc}s", 1_000.0),
    ("ms", 1_000_000.0),
    ("s", 1_000_000_000.0),
    ("m", 60_000_000_000.0),
    ("h", 3_600_000_000_000.0),
];

/// Parses a Go `time.Duration` string; `None` for anything else (including an empty string).
///
/// Sub-nanosecond digits are rounded. A bare `0` is zero, as in Go.
pub(super) fn parse_go_duration(text: &str) -> Option<SignedDuration> {
    let (negative, mut rest) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    if rest == "0" {
        return Some(SignedDuration::ZERO);
    }
    if rest.is_empty() {
        return None;
    }
    let mut nanos = 0f64;
    while !rest.is_empty() {
        let number_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let (number, tail) = rest.split_at(number_len);
        let value: f64 = number.parse().ok()?;
        let (unit, scale) = UNITS.iter().find(|(unit, _)| tail.starts_with(unit))?;
        nanos += value * scale;
        rest = &tail[unit.len()..];
    }
    if !nanos.is_finite() || nanos >= i64::MAX as f64 {
        return None;
    }
    let nanos = nanos.round() as i64;
    Some(SignedDuration::from_nanos(if negative {
        -nanos
    } else {
        nanos
    }))
}
