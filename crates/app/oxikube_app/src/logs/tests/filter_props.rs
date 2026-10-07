//! The incremental index equals a naive scan of the retained lines, whatever the appends, trims
//! (capacity changes) and chunked rescans.

use std::sync::Arc;

use proptest::prelude::*;

use super::line;
use crate::logs::{LogBuffer, LogEntry, LogFilter, MatchIndex};

#[derive(Debug, Clone)]
enum Op {
    Append(Vec<u8>),
    Capacity(usize),
    Scan(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => proptest::collection::vec(0u8..4, 0..30).prop_map(Op::Append),
        1 => (1usize..25).prop_map(Op::Capacity),
        3 => (1usize..40).prop_map(Op::Scan),
    ]
}

fn text(kind: u8) -> &'static str {
    ["plain", "an ERROR here", "WARN: Error-ish", "ünïcode Ünï"][kind as usize]
}

fn naive(buffer: &LogBuffer, matcher: &crate::logs::LogMatcher) -> Vec<u64> {
    buffer
        .iter()
        .filter(|e| matcher.matches(&e.text))
        .map(|e| e.seq)
        .collect()
}

proptest! {
    #[test]
    fn the_index_equals_a_naive_full_scan(
        ops in proptest::collection::vec(op(), 0..40),
        start in 1usize..25,
        pattern in prop_oneof![Just("error"), Just("^an"), Just("ünï"), Just("e.r"), Just("")],
        case_sensitive in any::<bool>(),
        inverse in any::<bool>(),
    ) {
        let matcher = Arc::new(
            LogFilter { pattern: pattern.to_owned(), case_sensitive, inverse }.compile().unwrap(),
        );
        let mut buffer = LogBuffer::new(start);
        let mut index = MatchIndex::new(matcher.clone());
        for op in ops {
            match op {
                Op::Append(kinds) => {
                    buffer.extend(kinds.iter().map(|k| {
                        let mut e = LogEntry::new(line(0));
                        e.text = Arc::from(text(*k));
                        e
                    }));
                }
                Op::Capacity(c) => { buffer.set_capacity(c); }
                Op::Scan(max) => { index.scan(&buffer, max); }
            }
            // After a full catch-up the index is exactly the naive answer.
            let mut full = index.clone();
            full.catch_up(&buffer);
            prop_assert_eq!(full.iter().collect::<Vec<_>>(), naive(&buffer, &matcher));
            prop_assert!(full.iter().all(|s| buffer.get_seq(s).is_some()), "no trimmed seq is kept");
            prop_assert!(full.is_caught_up(&buffer));
            // Sorted, and navigation stays on matches.
            let all: Vec<u64> = full.iter().collect();
            prop_assert!(all.windows(2).all(|w| w[0] < w[1]));
            if let Some(next) = full.next(None, buffer.first_seq()) {
                prop_assert!(full.contains(next));
            }
            if let Some(prev) = full.prev(None) {
                prop_assert!(full.contains(prev));
            }
        }
    }
}
