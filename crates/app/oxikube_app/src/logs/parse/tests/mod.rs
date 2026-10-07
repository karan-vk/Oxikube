//! Parser tests: the corpus of the four loggers the story names, mixed streams, levels, times and
//! the summary.

mod corpus;
mod mixed;
mod spellings;

use jiff::Timestamp;
use oxikube_domain::log::LogLevel;

use super::{LogRecord, parse_line};

/// The parsed lines of a fixture: every non-empty line must be a record.
fn records(log: &str) -> Vec<LogRecord> {
    log.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_line(line).unwrap_or_else(|| panic!("not a record: {line}")))
        .collect()
}

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn levels(records: &[LogRecord]) -> Vec<LogLevel> {
    records.iter().map(|r| r.level).collect()
}
