//! Top consumers: the pods and namespaces using the most CPU and memory, picked from a metrics
//! poll with a bounded heap ([`top_n`]), so a tick over 10 000 pods keeps `n` entries and never
//! sorts the whole list.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::sync::Arc;

use crate::metrics::ClusterUsage;

/// One row of a top list: a pod (`namespace` set) or a namespace, and its usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopEntry {
    /// The pod's namespace; `None` for a namespace row.
    pub namespace: Option<Arc<str>>,
    /// The pod's or the namespace's name.
    pub name: Arc<str>,
    /// CPU in nanocores or memory in bytes, whichever the list ranks.
    pub value: u64,
}

impl TopEntry {
    /// Rank: more first; for equal values, by namespace then name, so the order is stable.
    fn rank(&self, other: &Self) -> Ordering {
        self.value
            .cmp(&other.value)
            .then_with(|| other.namespace.cmp(&self.namespace))
            .then_with(|| other.name.cmp(&self.name))
    }
}

/// Orders entries by [`TopEntry::rank`] for the heap.
#[derive(Debug, PartialEq, Eq)]
struct Ranked(TopEntry);

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.rank(&other.0)
    }
}

/// A bounded min-heap: keeps the `n` largest entries pushed so far. A push costs O(log n), and
/// memory never grows past `n`, however many entries go by.
#[derive(Debug)]
pub struct TopHeap {
    n: usize,
    heap: BinaryHeap<Reverse<Ranked>>,
}

impl TopHeap {
    /// An empty heap that keeps `n` entries.
    pub fn new(n: usize) -> Self {
        Self {
            n,
            heap: BinaryHeap::with_capacity(n + 1),
        }
    }

    /// Offers `entry`: kept when the heap is not full or it ranks above the least kept.
    pub fn push(&mut self, entry: TopEntry) {
        if self.heap.len() < self.n {
            self.heap.push(Reverse(Ranked(entry)));
        } else if self
            .heap
            .peek()
            .is_some_and(|Reverse(least)| entry.rank(&least.0).is_gt())
        {
            self.heap.pop();
            self.heap.push(Reverse(Ranked(entry)));
        }
    }

    /// The kept entries, largest first.
    pub fn into_sorted(self) -> Vec<TopEntry> {
        let mut out: Vec<TopEntry> = self.heap.into_iter().map(|Reverse(r)| r.0).collect();
        out.sort_by(|a, b| b.rank(a));
        out
    }
}

/// The `n` largest of `entries`, largest first (ties by namespace then name). One pass over a
/// [`TopHeap`]: O(len log n).
pub fn top_n(entries: impl IntoIterator<Item = TopEntry>, n: usize) -> Vec<TopEntry> {
    let mut heap = TopHeap::new(n);
    entries.into_iter().for_each(|entry| heap.push(entry));
    heap.into_sorted()
}

/// The four top lists of a metrics poll.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TopConsumers {
    /// Pods by CPU.
    pub pods_cpu: Vec<TopEntry>,
    /// Pods by memory.
    pub pods_memory: Vec<TopEntry>,
    /// Namespaces by the CPU of their pods.
    pub namespaces_cpu: Vec<TopEntry>,
    /// Namespaces by the memory of their pods.
    pub namespaces_memory: Vec<TopEntry>,
}

/// The `n` largest pods and namespaces of `usage`'s latest pod readings, for CPU and memory.
/// One pass over the pods; namespaces are summed as their pods go by (they come grouped).
pub fn top_consumers(usage: &ClusterUsage, n: usize) -> TopConsumers {
    let mut pods_cpu = TopHeap::new(n);
    let mut pods_memory = TopHeap::new(n);
    let mut namespaces: Vec<(Arc<str>, u64, u64)> = Vec::new();
    usage.visit_pods(|namespace, name, point| {
        let entry = |value| TopEntry {
            namespace: Some(namespace.clone()),
            name: name.clone(),
            value,
        };
        let (cpu, memory) = (point.cpu_nanocores, point.memory_bytes);
        if let Some(cpu) = cpu {
            pods_cpu.push(entry(cpu));
        }
        if let Some(memory) = memory {
            pods_memory.push(entry(memory));
        }
        match namespaces.last_mut() {
            Some((ns, c, m)) if **ns == **namespace => {
                *c = c.saturating_add(cpu.unwrap_or(0));
                *m = m.saturating_add(memory.unwrap_or(0));
            }
            _ => namespaces.push((namespace.clone(), cpu.unwrap_or(0), memory.unwrap_or(0))),
        }
    });
    let namespace_entry = |ns: &Arc<str>, value: u64| TopEntry {
        namespace: None,
        name: ns.clone(),
        value,
    };
    TopConsumers {
        pods_cpu: pods_cpu.into_sorted(),
        pods_memory: pods_memory.into_sorted(),
        namespaces_cpu: top_n(
            namespaces.iter().map(|(ns, c, _)| namespace_entry(ns, *c)),
            n,
        ),
        namespaces_memory: top_n(
            namespaces.iter().map(|(ns, _, m)| namespace_entry(ns, *m)),
            n,
        ),
    }
}
