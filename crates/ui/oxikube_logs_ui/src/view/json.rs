//! JSON mode (E08-S05): a structured line drawn as level, time and message columns with its
//! other fields collapsed to `key=value`, and the caches that keep that cheap.
//!
//! The level of every line was read once, when the service committed it
//! ([`LogEntry::level`](oxikube_app::logs::LogEntry)); the columns are parsed here, only for the
//! rows on screen, and kept by line (seq) so a frame never parses a line it already drew. The
//! pretty-printed form is built when a line is expanded and cached the same way. Both caches are
//! bounded and start over with a new session (whose seqs start at 0 again).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui::SharedString;
use oxikube_app::logs::parse::{parse_line, pretty};
use oxikube_domain::log::LogLevel;

/// Rows whose columns are kept: several screens, so scrolling back and forth parses nothing.
const CACHED_ROWS: usize = 2_048;
/// Expanded lines whose pretty-printed text is kept.
const CACHED_PRETTY: usize = 16;
/// Characters of the collapsed `key=value` summary a row draws.
const SUMMARY_CHARS: usize = 400;
/// Lines of pretty-printed JSON the expanded pane holds (a 16 KiB line is far below it).
const MAX_PRETTY_LINES: usize = 2_000;

/// The columns of a JSON line, as the rows draw them (see [`LogView::row_columns`](super::LogView)),
/// ready to hand to the elements.
#[derive(Debug, PartialEq, Eq)]
pub struct JsonColumns {
    /// The normalised level ([`LogLevel::Unknown`] when the line names none).
    pub level: LogLevel,
    /// `HH:MM:SS.mmm` (UTC), or the line's own text when it could not be read as a time; empty
    /// without a time field.
    pub time: SharedString,
    /// The message on one line.
    pub message: SharedString,
    /// The other fields as `key=value`, cut to a few hundred characters.
    pub summary: SharedString,
}

/// The parsed rows and pretty-printed lines of one session, by seq.
#[derive(Default)]
pub(crate) struct RecordCache {
    rows: HashMap<u64, Arc<JsonColumns>>,
    /// Lines parsed over the cache's life (cache misses).
    parsed: usize,
    pretty: VecDeque<(u64, Arc<[SharedString]>)>,
}

impl RecordCache {
    /// The columns of line `seq` with this `text`, parsed on first use. `None` when the text is
    /// not a JSON object after all (a line the service classified as structured always is).
    pub fn row(&mut self, seq: u64, text: &str) -> Option<Arc<JsonColumns>> {
        if let Some(row) = self.rows.get(&seq) {
            return Some(row.clone());
        }
        let record = parse_line(text)?;
        self.parsed += 1;
        let row = Arc::new(JsonColumns {
            level: record.level,
            time: record
                .time
                .as_ref()
                .map(|time| time.display())
                .unwrap_or_default()
                .into(),
            message: record.message_line().into(),
            summary: record.summary(SUMMARY_CHARS).into(),
        });
        if self.rows.len() >= CACHED_ROWS {
            self.rows.clear();
        }
        self.rows.insert(seq, row.clone());
        Some(row)
    }

    /// The pretty-printed JSON of line `seq`, one entry per line, built on first use.
    pub fn pretty(&mut self, seq: u64, text: &str) -> Option<Arc<[SharedString]>> {
        if let Some((_, lines)) = self.pretty.iter().find(|(s, _)| *s == seq) {
            return Some(lines.clone());
        }
        let printed = pretty(text)?;
        let mut lines: Vec<SharedString> = printed
            .lines()
            .take(MAX_PRETTY_LINES + 1)
            .map(|line| SharedString::from(line.to_owned()))
            .collect();
        if lines.len() > MAX_PRETTY_LINES {
            lines[MAX_PRETTY_LINES] = "…".into();
        }
        let lines: Arc<[SharedString]> = lines.into();
        if self.pretty.len() >= CACHED_PRETTY {
            self.pretty.pop_front();
        }
        self.pretty.push_back((seq, lines.clone()));
        Some(lines)
    }

    /// Lines parsed so far.
    pub fn parsed(&self) -> usize {
        self.parsed
    }

    /// Forgets everything (a new session numbers its lines from 0 again).
    pub fn clear(&mut self) {
        self.rows.clear();
        self.pretty.clear();
    }

    /// Rows kept (for tests).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.rows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"{"level":"warn","ts":1696670400.5,"msg":"slow\nquery","ms":812}"#;

    #[test]
    fn a_row_is_parsed_once_and_then_served_from_the_cache() {
        let mut cache = RecordCache::default();
        let first = cache.row(7, LINE).unwrap();
        assert_eq!(first.level, LogLevel::Warn);
        assert_eq!(first.time.as_ref(), "09:20:00.500");
        assert_eq!(first.message.as_ref(), "slow query");
        assert_eq!(first.summary.as_ref(), "ms=812");
        // Served from the cache: the same allocation, even if the text asked for differs.
        let again = cache.row(7, "not even json").unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(cache.len(), 1);
        assert!(cache.row(8, "not even json").is_none());
    }

    #[test]
    fn the_row_cache_is_bounded() {
        let mut cache = RecordCache::default();
        for seq in 0..(CACHED_ROWS as u64 + 10) {
            cache.row(seq, r#"{"msg":"m"}"#).unwrap();
        }
        assert!(cache.len() <= CACHED_ROWS);
        cache.clear();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn pretty_text_is_built_per_line_id_and_cached() {
        let mut cache = RecordCache::default();
        let lines = cache.pretty(3, r#"{"a":1,"b":{"c":[1]}}"#).unwrap();
        assert_eq!(lines[0].as_ref(), "{");
        assert_eq!(lines[1].as_ref(), r#"  "a": 1,"#);
        assert_eq!(lines.last().unwrap().as_ref(), "}");
        let again = cache.pretty(3, "different text").unwrap();
        assert!(Arc::ptr_eq(&lines, &again), "cached by seq");
        assert!(cache.pretty(4, "plain").is_none());
        for seq in 10..40 {
            cache.pretty(seq, r#"{"x":1}"#).unwrap();
        }
        assert!(cache.pretty.len() <= CACHED_PRETTY);
    }

    #[test]
    fn a_huge_object_is_cut_to_a_bounded_number_of_lines() {
        let fields: Vec<String> = (0..3_000).map(|i| format!(r#""k{i}":{i}"#)).collect();
        let text = format!("{{{}}}", fields.join(","));
        let lines = RecordCache::default().pretty(0, &text).unwrap();
        assert_eq!(lines.len(), MAX_PRETTY_LINES + 1);
        assert_eq!(lines.last().unwrap().as_ref(), "…");
    }
}
