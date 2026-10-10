//! [`RecentList`]: most recent first, no repeats, a fixed capacity.

use std::collections::VecDeque;

/// A list of the things used lately: the latest first, each at most once, the oldest forgotten
/// beyond `capacity`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentList<T> {
    items: VecDeque<T>,
    capacity: usize,
}

impl<T: PartialEq> RecentList<T> {
    /// An empty list that keeps at most `capacity` items.
    pub fn new(capacity: usize) -> Self {
        Self {
            items: VecDeque::new(),
            capacity,
        }
    }

    /// Notes that `item` has just been used: it becomes the first. Returns whether the list
    /// changed (it did not if `item` was first already).
    pub fn touch(&mut self, item: T) -> bool {
        if self.items.front() == Some(&item) {
            return false;
        }
        self.items.retain(|existing| *existing != item);
        self.items.push_front(item);
        self.items.truncate(self.capacity);
        true
    }

    /// Adds `older` items behind the current ones, skipping those already present: how the stored
    /// list joins what was used before it was read. Returns whether the list changed.
    pub fn append_older(&mut self, older: impl IntoIterator<Item = T>) -> bool {
        let before = self.items.len();
        for item in older {
            if self.items.len() >= self.capacity {
                break;
            }
            if !self.items.contains(&item) {
                self.items.push_back(item);
            }
        }
        self.items.len() != before
    }

    /// Forgets everything. Returns whether there was anything.
    pub fn clear(&mut self) -> bool {
        let had = !self.items.is_empty();
        self.items.clear();
        had
    }

    /// The items, latest first.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    /// How many items are kept.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
