//! Lock-free single-producer ring buffer of `u64` samples (frame durations in nanoseconds).
//!
//! The producer is the UI thread; the reader is the flush thread. The producer never waits: when
//! the reader falls more than `capacity` samples behind, the oldest samples are overwritten and the
//! reader reports them as dropped. The protocol is a per-slot seqlock in spirit:
//!
//! - producer: publish `claimed = i + 1`, release fence, store slot `i % cap`, publish `head = i + 1`
//!   (release);
//! - reader: load `head` (acquire), copy `[next, head)`, acquire fence, load `claimed`; any copied
//!   index below `claimed - cap` may have been overwritten during the copy and is discarded as
//!   dropped.
//!
//! All slots are atomics, so there is no undefined behaviour even if the protocol is misused (for
//! example two producers); misuse can only produce wrong numbers.

use std::sync::atomic::{AtomicU64, Ordering, fence};

/// Fixed-capacity, single-producer, lock-free ring of `u64` samples.
pub struct FrameRing {
    slots: Box<[AtomicU64]>,
    mask: u64,
    /// Number of samples fully written (published).
    head: AtomicU64,
    /// Number of samples whose write has started (`head` or `head + 1`).
    claimed: AtomicU64,
}

impl FrameRing {
    /// A ring holding at least `capacity` samples (rounded up to a power of two, minimum 2).
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(2).next_power_of_two();
        Self {
            slots: (0..capacity).map(|_| AtomicU64::new(0)).collect(),
            mask: capacity as u64 - 1,
            head: AtomicU64::new(0),
            claimed: AtomicU64::new(0),
        }
    }

    /// Number of samples the ring holds before the oldest is overwritten.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Total samples pushed since creation.
    pub fn written(&self) -> u64 {
        self.head.load(Ordering::Acquire)
    }

    /// Appends a sample. Single producer only (the UI thread); never blocks or allocates.
    #[inline]
    pub fn push(&self, value: u64) {
        let index = self.head.load(Ordering::Relaxed);
        self.claimed.store(index + 1, Ordering::Relaxed);
        // Orders the claim before the slot write: a reader that sees the new slot value also sees
        // the claim and discards the overwritten sample.
        fence(Ordering::Release);
        self.slots[(index & self.mask) as usize].store(value, Ordering::Relaxed);
        self.head.store(index + 1, Ordering::Release);
    }

    /// A reader positioned at the current head (it sees only samples pushed from now on).
    pub fn reader_from_now(&self) -> RingReader {
        RingReader {
            next: self.written(),
        }
    }
}

/// Cursor over a [`FrameRing`]; each reader sees every sample once (or counts it as dropped).
#[derive(Debug, Default, Clone)]
pub struct RingReader {
    next: u64,
}

impl RingReader {
    /// A reader that starts at the first sample ever pushed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends every sample pushed since the last drain to `out` and returns how many were lost
    /// because the producer lapped the reader.
    pub fn drain_into(&mut self, ring: &FrameRing, out: &mut Vec<u64>) -> u64 {
        let capacity = ring.capacity() as u64;
        let head = ring.head.load(Ordering::Acquire);
        let mut start = self.next;
        let mut dropped = 0;
        if head - start > capacity {
            dropped += head - capacity - start;
            start = head - capacity;
        }
        let base = out.len();
        out.extend(
            (start..head).map(|i| ring.slots[(i & ring.mask) as usize].load(Ordering::Relaxed)),
        );
        fence(Ordering::Acquire);
        // Indices below `claimed - capacity` share a slot with a sample written (or being written)
        // after we loaded `head`: their copied value may be the newer one.
        let safe_from = ring
            .claimed
            .load(Ordering::Relaxed)
            .saturating_sub(capacity);
        if safe_from > start {
            let lost = (safe_from - start).min(head - start) as usize;
            out.drain(base..base + lost);
            dropped += lost as u64;
        }
        self.next = head;
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn capacity_rounds_up_to_power_of_two() {
        assert_eq!(FrameRing::with_capacity(0).capacity(), 2);
        assert_eq!(FrameRing::with_capacity(5).capacity(), 8);
        assert_eq!(FrameRing::with_capacity(4096).capacity(), 4096);
    }

    #[test]
    fn drains_in_order_and_incrementally() {
        let ring = FrameRing::with_capacity(8);
        let mut reader = RingReader::new();
        let mut out = Vec::new();
        for v in 1..=3 {
            ring.push(v);
        }
        assert_eq!(reader.drain_into(&ring, &mut out), 0);
        assert_eq!(out, [1, 2, 3]);
        out.clear();
        assert_eq!(reader.drain_into(&ring, &mut out), 0);
        assert!(out.is_empty(), "nothing new");
        ring.push(4);
        reader.drain_into(&ring, &mut out);
        assert_eq!(out, [4]);
    }

    #[test]
    fn wrap_around_keeps_newest_and_counts_dropped() {
        let ring = FrameRing::with_capacity(4);
        let mut reader = RingReader::new();
        for v in 0..10 {
            ring.push(v);
        }
        let mut out = Vec::new();
        let dropped = reader.drain_into(&ring, &mut out);
        assert_eq!(out, [6, 7, 8, 9], "the newest `capacity` samples survive");
        assert_eq!(dropped, 6);
        // After a lap the reader continues from the head without double counting.
        for v in 10..13 {
            ring.push(v);
        }
        out.clear();
        assert_eq!(reader.drain_into(&ring, &mut out), 0);
        assert_eq!(out, [10, 11, 12]);
    }

    #[test]
    fn exactly_full_ring_drops_nothing() {
        let ring = FrameRing::with_capacity(4);
        let mut reader = RingReader::new();
        for v in 0..4 {
            ring.push(v);
        }
        let mut out = Vec::new();
        assert_eq!(reader.drain_into(&ring, &mut out), 0);
        assert_eq!(out, [0, 1, 2, 3]);
    }

    #[test]
    fn reader_from_now_skips_history() {
        let ring = FrameRing::with_capacity(4);
        ring.push(1);
        let mut reader = ring.reader_from_now();
        ring.push(2);
        let mut out = Vec::new();
        reader.drain_into(&ring, &mut out);
        assert_eq!(out, [2]);
    }

    /// A producer thread pushing 0, 1, 2, ... against a reader draining concurrently: every value is
    /// either received exactly once, in order, or counted as dropped; no torn or stale values.
    #[test]
    fn concurrent_producer_and_reader_account_for_every_sample() {
        const TOTAL: u64 = 200_000;
        let ring = Arc::new(FrameRing::with_capacity(64));
        let producer = {
            let ring = ring.clone();
            std::thread::spawn(move || {
                for v in 0..TOTAL {
                    ring.push(v);
                }
            })
        };
        let mut reader = RingReader::new();
        let mut received = Vec::new();
        let mut dropped = 0;
        loop {
            let finished = producer.is_finished();
            dropped += reader.drain_into(&ring, &mut received);
            if finished && ring.written() == TOTAL {
                dropped += reader.drain_into(&ring, &mut received);
                break;
            }
        }
        producer.join().unwrap();
        assert_eq!(received.len() as u64 + dropped, TOTAL);
        assert!(
            received.windows(2).all(|w| w[0] < w[1]),
            "values strictly increase (no stale or duplicated slot reads)"
        );
        assert_eq!(*received.last().unwrap(), TOTAL - 1);
    }
}
