//! Level spellings: names (`"warning"`, `"ERR"`, `"critical"`) and the numeric levels of bunyan
//! and pino (10 trace, 20 debug, 30 info, 40 warn, 50 error, 60 fatal).

use oxikube_domain::log::LogLevel;
use serde_json::Value;

/// The normalised level of a JSON value: a name, or a number (bunyan and pino: the level is in
/// its decade, so 35 is info and 60 or more is fatal). Anything else, and numbers under 10, is
/// [`LogLevel::Unknown`].
pub fn normalise_level(value: &Value) -> LogLevel {
    match value {
        Value::String(name) => from_name(name),
        Value::Number(number) => number.as_f64().map_or(LogLevel::Unknown, from_number),
        _ => LogLevel::Unknown,
    }
}

fn from_number(n: f64) -> LogLevel {
    if !n.is_finite() || n < 10.0 {
        LogLevel::Unknown
    } else if n < 20.0 {
        LogLevel::Trace
    } else if n < 30.0 {
        LogLevel::Debug
    } else if n < 40.0 {
        LogLevel::Info
    } else if n < 50.0 {
        LogLevel::Warn
    } else if n < 60.0 {
        LogLevel::Error
    } else {
        LogLevel::Fatal
    }
}

fn from_name(name: &str) -> LogLevel {
    let name = name.trim();
    // Short names only: a "level" that is a sentence is not a level.
    if name.is_empty() || name.len() > 12 {
        return LogLevel::Unknown;
    }
    let mut lower = [0u8; 12];
    for (slot, byte) in lower.iter_mut().zip(name.bytes()) {
        *slot = byte.to_ascii_lowercase();
    }
    // The table comes before the number parse: `f64::from_str` accepts "inf" and "infinity", and
    // "inf" is a short name for info.
    let by_name = match &lower[..name.len()] {
        b"trace" | b"trc" | b"verbose" | b"silly" | b"finest" | b"finer" => LogLevel::Trace,
        b"debug" | b"dbg" | b"dev" | b"fine" => LogLevel::Debug,
        b"info" | b"inf" | b"information" | b"notice" | b"default" => LogLevel::Info,
        b"warn" | b"warning" | b"wrn" => LogLevel::Warn,
        b"error" | b"err" | b"eror" | b"severe" => LogLevel::Error,
        b"fatal" | b"ftl" | b"panic" | b"dpanic" | b"crit" | b"critical" | b"alert" | b"emerg"
        | b"emergency" => LogLevel::Fatal,
        _ => LogLevel::Unknown,
    };
    match (by_name, name.parse::<f64>()) {
        (LogLevel::Unknown, Ok(n)) => from_number(n),
        (level, _) => level,
    }
}
