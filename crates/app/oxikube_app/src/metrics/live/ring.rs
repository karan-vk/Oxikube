//! [`SampleRing`]: the last [`RING_CAPACITY`] readings of one node or pod, for sparklines.
//!
//! A ring is one allocation of fixed size made when its object is first sampled; a push
//! overwrites the oldest reading once it is full. A reading is packed into 24 bytes (seconds,
//! nanocores, bytes, with `u64::MAX` for a missing value), so a 10 000-pod cluster's rings take
//! about 14 MB. Two polls inside one metrics-server scrape return the same timestamp: the second
//! is not pushed again.

use jiff::Timestamp;
use oxikube_domain::Quantity;
use oxikube_domain::metrics::Reading;

/// Readings kept per object: the last 60, 15 minutes at the default interval.
pub const RING_CAPACITY: usize = 60;

/// The packed form of a missing value.
const MISSING: u64 = u64::MAX;

/// One reading of a node or pod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePoint {
    /// When metrics-server took it (whole seconds).
    pub at: Timestamp,
    /// CPU in nanocores, if read.
    pub cpu_nanocores: Option<u64>,
    /// Working-set memory in bytes, if read.
    pub memory_bytes: Option<u64>,
}

impl UsagePoint {
    /// A reading at `at` of `cpu` and `memory`.
    pub fn new(at: Timestamp, cpu: Reading, memory: Reading) -> Self {
        Self {
            at,
            cpu_nanocores: cpu.value(),
            memory_bytes: memory.value(),
        }
    }

    /// CPU as an exact quantity in cores.
    pub fn cpu_quantity(&self) -> Option<Quantity> {
        self.cpu_nanocores.map(cpu_quantity)
    }

    /// Memory as an exact quantity in bytes.
    pub fn memory_quantity(&self) -> Option<Quantity> {
        self.memory_bytes.map(memory_quantity)
    }
}

/// `nanocores` as an exact quantity in cores: the sort key of a CPU cell.
pub(super) fn cpu_quantity(nanocores: u64) -> Quantity {
    Quantity::from_nanos(i128::from(nanocores))
}

/// `bytes` as an exact quantity: the sort key of a memory cell.
pub(super) fn memory_quantity(bytes: u64) -> Quantity {
    Quantity::from_nanos(i128::from(bytes) * 1_000_000_000)
}

#[derive(Clone, Copy, Default)]
struct Packed {
    seconds: i64,
    cpu: u64,
    memory: u64,
}

impl Packed {
    fn of(point: &UsagePoint) -> Self {
        Self {
            seconds: point.at.as_second(),
            cpu: point.cpu_nanocores.unwrap_or(MISSING),
            memory: point.memory_bytes.unwrap_or(MISSING),
        }
    }

    fn point(self) -> UsagePoint {
        let value = |v: u64| (v != MISSING).then_some(v);
        UsagePoint {
            at: Timestamp::from_second(self.seconds).unwrap_or(Timestamp::UNIX_EPOCH),
            cpu_nanocores: value(self.cpu),
            memory_bytes: value(self.memory),
        }
    }
}

/// The last [`RING_CAPACITY`] readings of one object, oldest first.
pub struct SampleRing {
    slots: Box<[Packed; RING_CAPACITY]>,
    /// Where the next reading goes.
    head: usize,
    len: usize,
}

impl Default for SampleRing {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SampleRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SampleRing")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

impl SampleRing {
    /// An empty ring (its one allocation is made here).
    pub fn new() -> Self {
        Self {
            slots: Box::new([Packed::default(); RING_CAPACITY]),
            head: 0,
            len: 0,
        }
    }

    /// Adds `point`, overwriting the oldest reading when full. A point with the same timestamp
    /// as the newest replaces nothing and is dropped. Returns whether it was added.
    pub fn push(&mut self, point: UsagePoint) -> bool {
        if self.latest().is_some_and(|last| last.at == point.at) {
            return false;
        }
        self.slots[self.head] = Packed::of(&point);
        self.head = (self.head + 1) % RING_CAPACITY;
        self.len = (self.len + 1).min(RING_CAPACITY);
        true
    }

    /// The newest reading.
    pub fn latest(&self) -> Option<UsagePoint> {
        (self.len > 0).then(|| self.slots[(self.head + RING_CAPACITY - 1) % RING_CAPACITY].point())
    }

    /// How many readings it holds.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The readings, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = UsagePoint> + '_ {
        let start = (self.head + RING_CAPACITY - self.len) % RING_CAPACITY;
        (0..self.len).map(move |i| self.slots[(start + i) % RING_CAPACITY].point())
    }
}
