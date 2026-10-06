//! The data side of toasts: what a [`Toast`] says, and the [`ToastQueue`] that decides which
//! toasts are visible. No GPUI types except a few `SharedString`s and handlers, so the policy is
//! unit-testable without an app.

use std::{collections::VecDeque, rc::Rc, time::Duration};

use gpui::{App, SharedString, Window};

/// How important a toast is; picks its icon and colour and its default timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLevel {
    /// Neutral news ("Context refreshed").
    Info,
    /// Something worked ("Copied").
    Success,
    /// Needs a look but nothing failed.
    Warning,
    /// Something failed; stays until dismissed.
    Error,
}

impl ToastLevel {
    /// How long a toast of this level stays when it does not say: errors stay until dismissed.
    pub fn default_timeout(self) -> Option<Duration> {
        match self {
            ToastLevel::Info | ToastLevel::Success => Some(Duration::from_secs(4)),
            ToastLevel::Warning => Some(Duration::from_secs(8)),
            ToastLevel::Error => None,
        }
    }
}

pub(super) type ActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A button on a toast ("Retry", "Open logs").
#[derive(Clone)]
pub struct ToastAction {
    pub(super) label: SharedString,
    pub(super) handler: ActionHandler,
}

impl ToastAction {
    /// A button labelled `label`; clicking it runs `handler` and dismisses the toast.
    pub fn new(
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            handler: Rc::new(handler),
        }
    }
}

/// A message for the toast layer. Build one with [`Toast::info`] and friends.
#[derive(Clone)]
pub struct Toast {
    pub(super) key: Option<SharedString>,
    pub(super) level: ToastLevel,
    pub(super) title: Option<SharedString>,
    pub(super) message: SharedString,
    pub(super) actions: Vec<ToastAction>,
    pub(super) timeout: Option<Duration>,
}

impl Toast {
    /// A toast of `level` with `message` and the level's default timeout.
    pub fn new(level: ToastLevel, message: impl Into<SharedString>) -> Self {
        Self {
            key: None,
            level,
            title: None,
            message: message.into(),
            actions: Vec::new(),
            timeout: level.default_timeout(),
        }
    }

    /// An [`ToastLevel::Info`] toast.
    pub fn info(message: impl Into<SharedString>) -> Self {
        Self::new(ToastLevel::Info, message)
    }

    /// A [`ToastLevel::Success`] toast.
    pub fn success(message: impl Into<SharedString>) -> Self {
        Self::new(ToastLevel::Success, message)
    }

    /// A [`ToastLevel::Warning`] toast.
    pub fn warning(message: impl Into<SharedString>) -> Self {
        Self::new(ToastLevel::Warning, message)
    }

    /// An [`ToastLevel::Error`] toast.
    pub fn error(message: impl Into<SharedString>) -> Self {
        Self::new(ToastLevel::Error, message)
    }

    /// Deduplication key: showing a toast whose key matches a visible or waiting toast replaces
    /// that toast's content in place (and restarts its timeout) instead of stacking a copy.
    /// Use a stable key per cause, such as `"connect/prod-eu"`.
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A bold line above the message.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Adds an action button.
    pub fn action(mut self, action: ToastAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Dismisses itself after `timeout`.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Stays until the user (or code) dismisses it.
    pub fn persistent(mut self) -> Self {
        self.timeout = None;
        self
    }
}

/// Identifies a toast while it is shown or waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToastId(pub(super) u64);

impl ToastId {
    /// The id's number: it names the toast's `toast-<n>` element in tests and logs.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// A toast in the queue.
pub(super) struct Entry {
    pub(super) id: ToastId,
    /// Bumped whenever the toast is (re)shown, so an old timer cannot dismiss a newer showing.
    pub(super) generation: u64,
    pub(super) toast: Toast,
}

/// What [`ToastQueue::push`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pushed {
    pub(super) id: ToastId,
    /// The toast is on screen now (its timeout should start).
    pub(super) visible: bool,
}

/// Which toasts are visible (at most `max_visible`, oldest first) and which wait their turn.
pub(super) struct ToastQueue {
    pub(super) visible: Vec<Entry>,
    pending: VecDeque<Entry>,
    max_visible: usize,
    next_id: u64,
    next_generation: u64,
}

impl ToastQueue {
    pub(super) fn new(max_visible: usize) -> Self {
        Self {
            visible: Vec::new(),
            pending: VecDeque::new(),
            max_visible: max_visible.max(1),
            next_id: 0,
            next_generation: 0,
        }
    }

    pub(super) fn max_visible(&self) -> usize {
        self.max_visible
    }

    pub(super) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub(super) fn pending(&self) -> impl Iterator<Item = &Entry> {
        self.pending.iter()
    }

    fn generation(&mut self) -> u64 {
        self.next_generation += 1;
        self.next_generation
    }

    /// Adds `toast`, or replaces the toast with the same key.
    pub(super) fn push(&mut self, toast: Toast) -> Pushed {
        if let Some(key) = &toast.key {
            let generation = self.generation();
            if let Some(entry) = self
                .visible
                .iter_mut()
                .find(|entry| entry.toast.key.as_ref() == Some(key))
            {
                entry.toast = toast;
                entry.generation = generation;
                return Pushed {
                    id: entry.id,
                    visible: true,
                };
            }
            if let Some(entry) = self
                .pending
                .iter_mut()
                .find(|entry| entry.toast.key.as_ref() == Some(key))
            {
                entry.toast = toast;
                entry.generation = generation;
                return Pushed {
                    id: entry.id,
                    visible: false,
                };
            }
        }
        self.next_id += 1;
        let id = ToastId(self.next_id);
        let generation = self.generation();
        let entry = Entry {
            id,
            generation,
            toast,
        };
        let visible = self.visible.len() < self.max_visible;
        if visible {
            self.visible.push(entry);
        } else {
            self.pending.push_back(entry);
        }
        Pushed { id, visible }
    }

    /// Removes a toast (visible or waiting) and promotes waiting toasts into the free slots.
    /// Returns whether it existed, and the toasts that became visible.
    pub(super) fn dismiss(&mut self, id: ToastId) -> (bool, Vec<ToastId>) {
        let before = self.visible.len() + self.pending.len();
        self.visible.retain(|entry| entry.id != id);
        self.pending.retain(|entry| entry.id != id);
        let existed = self.visible.len() + self.pending.len() != before;
        (existed, self.promote())
    }

    /// Removes the toast with `key`.
    pub(super) fn dismiss_key(&mut self, key: &str) -> (Option<ToastId>, Vec<ToastId>) {
        let id = self
            .visible
            .iter()
            .chain(self.pending.iter())
            .find(|entry| entry.toast.key.as_deref() == Some(key))
            .map(|entry| entry.id);
        match id {
            Some(id) => (Some(id), self.dismiss(id).1),
            None => (None, Vec::new()),
        }
    }

    /// Changes the visible limit; returns the toasts that became visible.
    pub(super) fn set_max_visible(&mut self, max: usize) -> Vec<ToastId> {
        self.max_visible = max.max(1);
        // Over the new limit: the newest visible toasts go back to waiting (front of the queue,
        // keeping their order).
        while self.visible.len() > self.max_visible {
            if let Some(entry) = self.visible.pop() {
                self.pending.push_front(entry);
            }
        }
        self.promote()
    }

    fn promote(&mut self) -> Vec<ToastId> {
        let mut promoted = Vec::new();
        while self.visible.len() < self.max_visible {
            let Some(mut entry) = self.pending.pop_front() else {
                break;
            };
            entry.generation = self.generation();
            promoted.push(entry.id);
            self.visible.push(entry);
        }
        promoted
    }

    pub(super) fn get(&self, id: ToastId) -> Option<&Entry> {
        self.visible.iter().find(|entry| entry.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(key: &str, message: &str) -> Toast {
        Toast::info(message.to_owned()).key(key.to_owned())
    }

    fn visible_messages(queue: &ToastQueue) -> Vec<String> {
        queue
            .visible
            .iter()
            .map(|e| e.toast.message.to_string())
            .collect()
    }

    #[test]
    fn extra_toasts_wait_and_are_promoted_in_order() {
        let mut queue = ToastQueue::new(2);
        let a = queue.push(Toast::info("a"));
        let b = queue.push(Toast::info("b"));
        let c = queue.push(Toast::info("c"));
        let d = queue.push(Toast::info("d"));
        assert!(a.visible && b.visible && !c.visible && !d.visible);
        assert_eq!(visible_messages(&queue), ["a", "b"]);
        assert_eq!(queue.pending_len(), 2);

        let (existed, promoted) = queue.dismiss(a.id);
        assert!(existed);
        assert_eq!(promoted, [c.id]);
        assert_eq!(visible_messages(&queue), ["b", "c"]);
        assert_eq!(queue.pending_len(), 1);
    }

    #[test]
    fn same_key_replaces_in_place_and_restarts() {
        let mut queue = ToastQueue::new(2);
        let first = queue.push(keyed("k", "one"));
        let generation = queue.get(first.id).unwrap().generation;
        let second = queue.push(keyed("k", "two"));
        assert_eq!(second.id, first.id);
        assert!(second.visible);
        assert_eq!(visible_messages(&queue), ["two"]);
        assert!(queue.get(first.id).unwrap().generation > generation);
    }

    #[test]
    fn same_key_replaces_a_waiting_toast_without_showing_it() {
        let mut queue = ToastQueue::new(1);
        queue.push(Toast::info("front"));
        let waiting = queue.push(keyed("k", "old"));
        let again = queue.push(keyed("k", "new"));
        assert!(!waiting.visible && !again.visible);
        assert_eq!(again.id, waiting.id);
        assert_eq!(queue.pending().next().unwrap().toast.message, "new");
        assert_eq!(queue.pending_len(), 1);
    }

    #[test]
    fn unkeyed_toasts_never_dedupe() {
        let mut queue = ToastQueue::new(5);
        queue.push(Toast::info("same"));
        queue.push(Toast::info("same"));
        assert_eq!(queue.visible.len(), 2);
    }

    #[test]
    fn dismiss_by_key_and_unknown_ids() {
        let mut queue = ToastQueue::new(1);
        let a = queue.push(keyed("a", "a"));
        let b = queue.push(keyed("b", "b"));
        let (id, promoted) = queue.dismiss_key("a");
        assert_eq!(id, Some(a.id));
        assert_eq!(promoted, [b.id]);
        assert_eq!(queue.dismiss_key("zzz"), (None, vec![]));
        assert_eq!(queue.dismiss(a.id), (false, vec![]));
    }

    #[test]
    fn lowering_the_limit_parks_the_newest_and_raising_it_promotes() {
        let mut queue = ToastQueue::new(3);
        let ids: Vec<_> = ["a", "b", "c"]
            .map(|m| queue.push(Toast::info(m)).id)
            .into();
        assert!(queue.set_max_visible(1).is_empty());
        assert_eq!(visible_messages(&queue), ["a"]);
        assert_eq!(queue.pending_len(), 2);
        let promoted = queue.set_max_visible(3);
        assert_eq!(promoted, [ids[1], ids[2]]);
        assert_eq!(visible_messages(&queue), ["a", "b", "c"]);
    }

    #[test]
    fn level_defaults() {
        assert_eq!(
            ToastLevel::Info.default_timeout(),
            Some(Duration::from_secs(4))
        );
        assert_eq!(ToastLevel::Error.default_timeout(), None);
        assert_eq!(
            Toast::error("x").timeout(Duration::from_secs(1)).timeout,
            Some(Duration::from_secs(1))
        );
        assert_eq!(Toast::info("x").persistent().timeout, None);
    }
}
