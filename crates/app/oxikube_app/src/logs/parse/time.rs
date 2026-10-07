//! Time spellings: RFC 3339 and close relatives, epoch seconds, milliseconds, microseconds and
//! nanoseconds, integers and floats.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde_json::Value;

/// Numbers below this are not epoch times (a duration like `0.5`, a counter): 1973 in seconds.
const MIN_EPOCH_SECONDS: f64 = 1e8;
/// Epoch seconds up to year 5138, milliseconds up to 1e14 (year 5138), then microseconds, then
/// nanoseconds: the unit is read from the magnitude, which the next two decades cannot confuse.
const SECONDS_BELOW: f64 = 1e11;
const MILLIS_BELOW: f64 = 1e14;
const MICROS_BELOW: f64 = 1e17;
const NANOS_BELOW: f64 = 1e20;

/// The instant a JSON value names: an RFC 3339 string (a zone-less `2026-10-07 12:00:00` is read
/// as UTC), a numeric string, or a number of epoch seconds, milliseconds, microseconds or
/// nanoseconds. `None` when it is none of these (the caller keeps the original text).
pub fn parse_time(value: &Value) -> Option<Timestamp> {
    match value {
        Value::Number(number) => match number.as_i64() {
            Some(n) => from_epoch_int(n),
            None => from_epoch_float(number.as_f64()?),
        },
        Value::String(text) => from_text(text.trim()),
        _ => None,
    }
}

fn from_text(text: &str) -> Option<Timestamp> {
    if text.is_empty() {
        return None;
    }
    if let Ok(timestamp) = text.parse::<Timestamp>() {
        return Some(timestamp);
    }
    if let Ok(n) = text.parse::<i64>() {
        return from_epoch_int(n);
    }
    if let Ok(n) = text.parse::<f64>() {
        return from_epoch_float(n);
    }
    // No offset (`2026-10-07 12:00:00.123`): UTC.
    let civil = text.parse::<DateTime>().ok()?;
    civil.to_zoned(TimeZone::UTC).ok().map(|z| z.timestamp())
}

/// Whole numbers are converted exactly (no float rounding of a millisecond count).
fn from_epoch_int(n: i64) -> Option<Timestamp> {
    let n = i128::from(n);
    if n < MIN_EPOCH_SECONDS as i128 {
        return None;
    }
    let nanos = if n < SECONDS_BELOW as i128 {
        n * 1_000_000_000
    } else if n < MILLIS_BELOW as i128 {
        n * 1_000_000
    } else if n < MICROS_BELOW as i128 {
        n * 1_000
    } else if n < NANOS_BELOW as i128 {
        n
    } else {
        return None;
    };
    Timestamp::from_nanosecond(nanos).ok()
}

/// Fractional epoch times (zap's `1696672800.123456`): good to a few hundred nanoseconds.
fn from_epoch_float(n: f64) -> Option<Timestamp> {
    if !n.is_finite() || !(MIN_EPOCH_SECONDS..NANOS_BELOW).contains(&n) {
        return None;
    }
    let nanos = if n < SECONDS_BELOW {
        n * 1e9
    } else if n < MILLIS_BELOW {
        n * 1e6
    } else if n < MICROS_BELOW {
        n * 1e3
    } else {
        n
    };
    // The ranges above keep `nanos` inside i128.
    Timestamp::from_nanosecond(nanos.round() as i128).ok()
}
