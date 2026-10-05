//! The reopen-closed stack: descriptors of recently closed items, never live entities.

use std::collections::VecDeque;

use gpui::SharedString;

use crate::pane::PaneId;

/// How many closed items the workspace remembers by default.
pub const CLOSED_ITEMS_CAPACITY: usize = 32;

/// What is kept of a closed item: enough to rebuild it through the
/// [`ItemRegistry`](crate::ItemRegistry) and put it back where it was.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedItem {
    /// The item's [`Item::serialized_kind`](crate::Item::serialized_kind).
    pub kind: &'static str,
    /// The item's [`Item::serialize`](crate::Item::serialize) output when it closed.
    pub state: serde_json::Value,
    /// The tab title when it closed (for menus such as "Reopen `title`").
    pub title: SharedString,
    /// The pane it was in, if known.
    pub pane: Option<PaneId>,
    /// Its tab index in that pane, if known.
    pub index: Option<usize>,
}

/// A bounded LIFO of [`ClosedItem`]s: pushing past the capacity forgets the oldest entry.
#[derive(Clone, Debug)]
pub struct ClosedItemStack {
    capacity: usize,
    entries: VecDeque<ClosedItem>,
}

impl ClosedItemStack {
    /// An empty stack holding at most `capacity` entries (at least one).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    /// Remembers `item` as the most recently closed one.
    pub fn push(&mut self, item: ClosedItem) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(item);
    }

    /// Takes the most recently closed item.
    pub fn pop(&mut self) -> Option<ClosedItem> {
        self.entries.pop_back()
    }

    /// The most recently closed item, without taking it.
    pub fn peek(&self) -> Option<&ClosedItem> {
        self.entries.back()
    }

    /// Number of remembered items.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The most entries kept.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for ClosedItemStack {
    fn default() -> Self {
        Self::new(CLOSED_ITEMS_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(n: usize) -> ClosedItem {
        ClosedItem {
            kind: "test",
            state: serde_json::json!(n),
            title: format!("item {n}").into(),
            pane: None,
            index: None,
        }
    }

    #[test]
    fn pops_most_recent_first() {
        let mut stack = ClosedItemStack::new(4);
        stack.push(closed(1));
        stack.push(closed(2));
        assert_eq!(
            stack.peek().map(|c| c.state.clone()),
            Some(serde_json::json!(2))
        );
        assert_eq!(stack.pop().map(|c| c.state), Some(serde_json::json!(2)));
        assert_eq!(stack.pop().map(|c| c.state), Some(serde_json::json!(1)));
        assert!(stack.pop().is_none());
    }

    #[test]
    fn is_bounded_and_forgets_the_oldest() {
        let mut stack = ClosedItemStack::new(3);
        for n in 0..10 {
            stack.push(closed(n));
            assert!(stack.len() <= 3);
        }
        let kept: Vec<_> = std::iter::from_fn(|| stack.pop())
            .map(|c| c.state)
            .collect();
        assert_eq!(
            kept,
            [
                serde_json::json!(9),
                serde_json::json!(8),
                serde_json::json!(7)
            ]
        );
    }

    #[test]
    fn zero_capacity_still_keeps_the_last_item() {
        let mut stack = ClosedItemStack::new(0);
        stack.push(closed(1));
        stack.push(closed(2));
        assert_eq!(stack.capacity(), 1);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.pop().map(|c| c.state), Some(serde_json::json!(2)));
    }

    #[test]
    fn default_capacity() {
        let mut stack = ClosedItemStack::default();
        for n in 0..CLOSED_ITEMS_CAPACITY + 5 {
            stack.push(closed(n));
        }
        assert_eq!(stack.len(), CLOSED_ITEMS_CAPACITY);
    }
}
