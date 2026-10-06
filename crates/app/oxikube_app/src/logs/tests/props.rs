//! Property tests: any sequence of appends and capacity changes keeps the buffer consistent with
//! a naive `VecDeque` model, and the window rebuilt from deltas equals the buffer.

use std::collections::VecDeque;
use std::task::{Context, Poll};

use futures::Stream;
use futures::task::noop_waker;
use oxikube_ports::LogOptions;
use proptest::prelude::*;

use super::line;
use crate::logs::shared::Shared;
use crate::logs::{LogBuffer, LogDeltas, LogEntry, LogTarget};

#[derive(Debug, Clone)]
enum Op {
    Append(usize),
    Capacity(usize),
    Poll,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0usize..60).prop_map(Op::Append),
        1 => (1usize..40).prop_map(Op::Capacity),
        2 => Just(Op::Poll),
    ]
}

proptest! {
    #[test]
    fn the_ring_matches_a_naive_model(ops in proptest::collection::vec(op(), 0..60), start in 1usize..30) {
        let mut buffer = LogBuffer::new(start);
        let mut model: VecDeque<u64> = VecDeque::new();
        let (mut capacity, mut next, mut dropped) = (start, 0u64, 0u64);
        for op in ops {
            match op {
                Op::Append(n) => {
                    buffer.extend((0..n).map(|i| LogEntry::new(line(i))));
                    for _ in 0..n { model.push_back(next); next += 1; }
                }
                Op::Capacity(c) => {
                    capacity = c;
                    buffer.set_capacity(c);
                }
                Op::Poll => {}
            }
            while model.len() > capacity { model.pop_front(); dropped += 1; }

            prop_assert_eq!(buffer.len(), model.len());
            prop_assert!(buffer.len() <= buffer.capacity());
            prop_assert_eq!(buffer.dropped(), dropped);
            prop_assert_eq!(buffer.next_seq(), next);
            prop_assert_eq!(buffer.first_seq(), next - model.len() as u64);
            for (index, seq) in model.iter().enumerate() {
                prop_assert_eq!(buffer.get(index).map(|e| e.seq), Some(*seq));
                prop_assert_eq!(buffer.index_of(*seq), Some(index));
                prop_assert_eq!(buffer.get_seq(*seq).map(|e| e.seq), Some(*seq));
            }
            prop_assert!(buffer.get(model.len()).is_none());
            let all: Vec<u64> = buffer.range(0..usize::MAX).map(|e| e.seq).collect();
            prop_assert_eq!(all, model.iter().copied().collect::<Vec<_>>());
        }
    }

    #[test]
    fn the_window_rebuilt_from_deltas_equals_the_buffer(
        ops in proptest::collection::vec(op(), 0..60),
        start in 1usize..30,
    ) {
        let shared = std::sync::Arc::new(Shared::new(
            1,
            LogTarget::pod("default", "web-0"),
            LogOptions::default(),
            start,
        ));
        let setting = std::sync::atomic::AtomicUsize::new(start);
        let mut deltas = LogDeltas::new(shared.clone());
        let (mut first, mut end) = (0u64, 0u64);
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        for op in ops {
            match op {
                Op::Append(n) => shared.commit((0..n).map(|i| LogEntry::new(line(i))).collect(), &setting),
                Op::Capacity(c) => shared.set_capacity(c),
                Op::Poll => {}
            }
            // Poll after every op for `Poll`, and occasionally after others via the next Poll.
            if matches!(op, Op::Poll) {
                if let Poll::Ready(Some(delta)) = std::pin::Pin::new(&mut deltas).poll_next(&mut cx) {
                    let len = end - first;
                    let new_len = len - delta.dropped_front as u64 + (delta.appended.end - delta.appended.start);
                    first = delta.first_seq;
                    end = delta.appended.end;
                    prop_assert_eq!(end - first, new_len);
                }
                shared.read(|buffer, _| {
                    prop_assert_eq!((first, end), (buffer.first_seq(), buffer.next_seq()));
                    Ok(())
                })?;
            }
        }
    }
}
