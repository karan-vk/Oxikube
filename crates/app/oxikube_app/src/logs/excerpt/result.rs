//! [`LogExcerpt`]: the text of a bounded read and the facts a reader needs to trust it.

use jiff::Timestamp;

/// The result of [`LogService::read_excerpt`](crate::logs::LogService::read_excerpt): the newest
/// matching lines as redacted text, and what was left out and why.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogExcerpt {
    /// `timestamp pod/container text` per line, oldest first, secrets masked (best effort: see
    /// [`oxikube_domain::redact`]). At most the request's byte budget.
    pub text: String,
    /// Lines in [`text`](Self::text).
    pub lines: usize,
    /// Lines that matched the filter among those read.
    pub matched: usize,
    /// Lines read and tested.
    pub scanned: usize,
    /// Streams (containers) read: 1 for a pod, one per container of every pod for a workload.
    pub streams: usize,
    /// Older matching lines left out because the tail (or the budget) was reached.
    pub omitted: usize,
    /// Whether older lines were dropped to fit the byte budget (not just the tail).
    pub budget_cut: bool,
    /// Lines read from each stream when a filter was given; the newest lines only were searched
    /// when `scanned` reached it.
    pub scan_limit: Option<usize>,
    /// Lines the session's buffer dropped (`logs.buffer_lines`) before they could be read.
    pub buffer_dropped: u64,
    /// The read did not finish before its deadline; what had arrived is here.
    pub timed_out: bool,
    /// Streams that failed (one line each, redacted), the rest were read.
    pub failures: Vec<String>,
    /// Pods a workload read left out (`logs.max_streams`).
    pub skipped_pods: usize,
    /// Pods the workload's selector matched; `None` for a single pod.
    pub matched_pods: Option<usize>,
    /// Server time of the first and last returned line.
    pub span: Option<(Timestamp, Timestamp)>,
}

impl LogExcerpt {
    /// Whether no line came back.
    pub fn is_empty(&self) -> bool {
        self.lines == 0
    }

    /// What the reader must know to interpret [`text`](Self::text), one sentence each: lines
    /// omitted, a search limited to the newest lines, a timeout, failed streams, skipped pods.
    /// Empty when the excerpt is complete.
    pub fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.omitted > 0 {
            let why = if self.budget_cut {
                "to fit the size limit"
            } else {
                "beyond the requested tail"
            };
            notes.push(format!(
                "{} older matching lines were omitted {why}; narrow with since or grep, or raise tail.",
                self.omitted
            ));
        } else if self.budget_cut {
            notes.push("Older lines were omitted to fit the size limit.".to_owned());
        }
        if let Some(limit) = self.scan_limit
            && self.scanned >= limit
        {
            notes.push(format!(
                "Only the newest {limit} lines of each container were searched; use since to reach further back."
            ));
        }
        if self.buffer_dropped > 0 {
            notes.push(format!(
                "{} older lines were dropped by the log buffer (logs.buffer_lines) before they could be read.",
                self.buffer_dropped
            ));
        }
        if self.timed_out {
            notes.push("The read hit its time limit; later lines may be missing.".to_owned());
        }
        for failure in &self.failures {
            notes.push(format!("Could not read {failure}."));
        }
        if self.skipped_pods > 0 {
            notes.push(format!(
                "{} more pods matched but were not read (logs.max_streams); narrow with a selector.",
                self.skipped_pods
            ));
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_excerpt_has_no_notes() {
        let excerpt = LogExcerpt {
            lines: 3,
            matched: 3,
            scanned: 3,
            streams: 1,
            ..LogExcerpt::default()
        };
        assert!(excerpt.notes().is_empty());
    }

    #[test]
    fn every_loss_is_named() {
        let excerpt = LogExcerpt {
            omitted: 5,
            budget_cut: true,
            scan_limit: Some(10),
            scanned: 10,
            buffer_dropped: 2,
            timed_out: true,
            failures: vec!["web-1/app: forbidden".into()],
            skipped_pods: 4,
            ..LogExcerpt::default()
        };
        let notes = excerpt.notes().join("\n");
        for needle in [
            "5 older matching lines were omitted to fit the size limit",
            "newest 10 lines",
            "2 older lines were dropped",
            "time limit",
            "Could not read web-1/app: forbidden",
            "4 more pods",
        ] {
            assert!(notes.contains(needle), "{needle}\n{notes}");
        }
    }
}
