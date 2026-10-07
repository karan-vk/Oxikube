//! [`ExcerptRequest`]: what a bounded, non-following read asks for, and the limits that keep it
//! bounded.

use std::time::Duration;

use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiError, OxiResult};

use super::super::{AggregateSpec, LogFilter};

/// Lines an excerpt returns when the caller names no `tail`.
pub const DEFAULT_TAIL: usize = 200;
/// The most lines one excerpt returns, whatever the caller asks.
pub const MAX_TAIL: usize = 2_000;
/// Lines read from each stream when the excerpt filters (`grep`): the newest lines are searched,
/// not the whole log. Without a filter exactly `tail` lines are read.
pub const SCAN_LINES: usize = 10_000;
/// The most bytes of text one excerpt returns (256 KiB): a tool call cannot flood an agent.
pub const MAX_EXCERPT_BYTES: usize = 256 * 1024;
/// The longest an excerpt waits for the read to finish; what arrived by then is returned.
pub const READ_DEADLINE: Duration = Duration::from_secs(20);
/// The furthest back `since` may reach (7 days).
pub const MAX_SINCE: Duration = Duration::from_secs(7 * 24 * 3_600);

/// What an excerpt reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcerptSource {
    /// One pod's container (the pod's default when `container` is `None`).
    Pod {
        /// Namespace of the pod.
        namespace: String,
        /// Pod name.
        pod: String,
        /// Container name.
        container: Option<String>,
    },
    /// The pods of a workload, a Service or a label selector, merged by server timestamp.
    Workload(AggregateSpec),
}

/// A bounded read of a log: the newest `tail` matching lines, optionally since a point in time and
/// matching a pattern, within a byte budget. It never follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptRequest {
    /// What is read.
    pub source: ExcerptSource,
    /// Only lines newer than this long ago (`sinceSeconds`).
    pub since: Option<Duration>,
    /// The most lines returned (the newest matching ones), at most [`MAX_TAIL`].
    pub tail: usize,
    /// Only the lines this matches (the viewer's search predicate).
    pub filter: LogFilter,
    /// The most bytes returned, at most [`MAX_EXCERPT_BYTES`].
    pub max_bytes: usize,
}

impl ExcerptRequest {
    /// A read of `source` with the default tail and budget.
    pub fn new(source: ExcerptSource) -> Self {
        Self {
            source,
            since: None,
            tail: DEFAULT_TAIL,
            filter: LogFilter::default(),
            max_bytes: MAX_EXCERPT_BYTES,
        }
    }

    /// Only lines newer than `since` ago.
    #[must_use]
    pub fn since(mut self, since: Duration) -> Self {
        self.since = Some(since);
        self
    }

    /// Returns at most `tail` lines (clamped to `1..=`[`MAX_TAIL`]).
    #[must_use]
    pub fn tail(mut self, tail: usize) -> Self {
        self.tail = tail.clamp(1, MAX_TAIL);
        self
    }

    /// Only lines `filter` matches.
    #[must_use]
    pub fn matching(mut self, filter: LogFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Returns at most `bytes` of text (clamped to [`MAX_EXCERPT_BYTES`]).
    #[must_use]
    pub fn max_bytes(mut self, bytes: usize) -> Self {
        self.max_bytes = bytes.min(MAX_EXCERPT_BYTES);
        self
    }

    /// Lines read from each stream: [`SCAN_LINES`] when filtering, else the tail.
    pub(super) fn scan_lines(&self) -> usize {
        if self.filter.is_empty() {
            self.tail.clamp(1, MAX_TAIL)
        } else {
            SCAN_LINES
        }
    }
}

/// The workload or Service kind a short or long name stands for: `deployment` / `deploy`,
/// `statefulset` / `sts`, `daemonset` / `ds`, `replicaset` / `rs`, `job`, `service` / `svc`.
/// `None` for anything else (a pod is not a workload: it is read directly).
pub fn workload_kind(word: &str) -> Option<Gvk> {
    let apps = |kind: &str| Gvk::new("apps", "v1", kind);
    Some(match word.to_ascii_lowercase().as_str() {
        "deployment" | "deploy" => apps("Deployment"),
        "statefulset" | "sts" => apps("StatefulSet"),
        "daemonset" | "ds" => apps("DaemonSet"),
        "replicaset" | "rs" => apps("ReplicaSet"),
        "job" => Gvk::new("batch", "v1", "Job"),
        "service" | "svc" => Gvk::new("", "v1", "Service"),
        _ => return None,
    })
}

/// Parses a `since` argument: one or more `<number><unit>` groups with the units `s`, `m`, `h`
/// and `d` (`90s`, `10m`, `2h`, `1h30m`, `1d`).
///
/// # Errors
///
/// A validation error for anything else, for zero, and for more than [`MAX_SINCE`].
pub fn parse_since(text: &str) -> OxiResult<Duration> {
    let bad = || {
        OxiError::validation(format!(
            "since {text:?}: expected a duration like 90s, 10m, 2h, 1h30m or 1d"
        ))
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(bad());
    }
    let mut total = 0u64;
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit: u64 = match c {
            's' => 1,
            'm' => 60,
            'h' => 3_600,
            'd' => 86_400,
            _ => return Err(bad()),
        };
        let amount: u64 = digits.parse().map_err(|_| bad())?;
        digits.clear();
        total = amount
            .checked_mul(unit)
            .and_then(|secs| total.checked_add(secs))
            .ok_or_else(bad)?;
    }
    if !digits.is_empty() || total == 0 {
        return Err(bad());
    }
    let since = Duration::from_secs(total);
    if since > MAX_SINCE {
        return Err(OxiError::validation(format!(
            "since {text:?} reaches back further than 7 days"
        )));
    }
    Ok(since)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_parses_units_and_sums_groups() {
        for (text, secs) in [
            ("90s", 90),
            ("10m", 600),
            ("2h", 7_200),
            ("1h30m", 5_400),
            ("1d", 86_400),
            (" 5m ", 300),
        ] {
            assert_eq!(parse_since(text).unwrap().as_secs(), secs, "{text}");
        }
    }

    #[test]
    fn since_rejects_nonsense_zero_and_too_far_back() {
        for bad in [
            "",
            "m",
            "10",
            "0s",
            "1x",
            "-5m",
            "1.5h",
            "8d",
            "99999999999999999999d",
        ] {
            assert!(parse_since(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_request_clamps_its_limits() {
        let request = ExcerptRequest::new(ExcerptSource::Pod {
            namespace: "default".into(),
            pod: "web".into(),
            container: None,
        })
        .tail(1_000_000)
        .max_bytes(usize::MAX);
        assert_eq!(request.tail, MAX_TAIL);
        assert_eq!(request.max_bytes, MAX_EXCERPT_BYTES);
        assert_eq!(request.scan_lines(), MAX_TAIL);
        assert_eq!(
            request.matching(LogFilter::new("err")).scan_lines(),
            SCAN_LINES
        );
    }
}
