//! [`TestDelegate`]: a picker delegate over a list of strings that records what the picker asked of
//! it, for this crate's `#[gpui::test]`s, its screenshot test and its bench (feature
//! `test-support`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Context, DismissEvent, InteractiveElement as _, SharedString, Stateful, Task, Window,
};

use super::fuzzy::{self, StringMatch, StringMatchCandidate};
use super::{Picker, PickerDelegate, match_label};

/// What a [`TestDelegate`] saw, shared so a test can read it after the picker closed.
#[derive(Clone, Default)]
pub struct TestRecord {
    /// Every confirm: the confirmed item's text and whether it was the secondary confirm.
    pub confirmed: Rc<RefCell<Vec<(String, bool)>>>,
    /// How many times [`PickerDelegate::dismissed`] ran.
    pub dismissed: Rc<Cell<usize>>,
    /// How many rows [`PickerDelegate::render_match`] built since the last reset.
    pub rendered: Rc<Cell<usize>>,
    /// Every query [`PickerDelegate::update_matches`] was asked for, in order.
    pub queries: Rc<RefCell<Vec<String>>>,
}

/// A delegate over `items`, fuzzy-matched with [`fuzzy::match_strings_async`] (so lists above
/// [`fuzzy::INLINE_MATCH_LIMIT`] match on the background executor). Confirming records the item
/// and closes the picker. A query listed in [`Self::slow`] takes that long to match, after which
/// it writes its matches unconditionally: whether a stale result can land is the picker's to
/// prevent.
pub struct TestDelegate {
    candidates: Arc<[StringMatchCandidate]>,
    matches: Vec<StringMatch>,
    selected: usize,
    delays: HashMap<String, Duration>,
    unselectable: Vec<usize>,
    record: TestRecord,
}

impl TestDelegate {
    /// A delegate over `items`, every one matching the empty query, the first selected.
    pub fn new<S: Into<SharedString>>(items: impl IntoIterator<Item = S>) -> Self {
        let candidates: Arc<[StringMatchCandidate]> = items
            .into_iter()
            .enumerate()
            .map(|(ix, item)| StringMatchCandidate::new(ix, item))
            .collect();
        let matches = fuzzy::match_strings(&candidates, "", usize::MAX);
        Self {
            candidates,
            matches,
            selected: 0,
            delays: HashMap::new(),
            unselectable: Vec::new(),
            record: TestRecord::default(),
        }
    }

    /// `n` items named `item-0000` .. (zero-padded so the order is the name order).
    pub fn numbered(n: usize) -> Self {
        Self::new((0..n).map(|ix| format!("item-{ix:04}")))
    }

    /// Makes matching `query` take `delay`.
    pub fn slow(mut self, query: &str, delay: Duration) -> Self {
        self.delays.insert(query.to_owned(), delay);
        self
    }

    /// Makes the matches of these candidates impossible to select (headers, say).
    pub fn unselectable(mut self, candidate_ids: impl IntoIterator<Item = usize>) -> Self {
        self.unselectable.extend(candidate_ids);
        self
    }

    /// What the delegate saw.
    pub fn record(&self) -> TestRecord {
        self.record.clone()
    }

    /// The text of the current matches, in order.
    pub fn match_texts(&self) -> Vec<String> {
        self.matches.iter().map(|m| m.string.to_string()).collect()
    }

    /// The text of the selected match.
    pub fn selected_text(&self) -> Option<String> {
        self.matches
            .get(self.selected)
            .map(|m| m.string.to_string())
    }
}

impl PickerDelegate for TestDelegate {
    type ListItem = Stateful<gpui::Div>;

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.selected = ix;
    }

    fn can_select(&self, ix: usize, _: &mut Window, _: &mut Context<Picker<Self>>) -> bool {
        self.matches
            .get(ix)
            .is_some_and(|m| !self.unselectable.contains(&m.candidate_id))
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Search items".into()
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        self.record.queries.borrow_mut().push(query.clone());
        let delay = self.delays.get(&query).copied();
        let candidates = self.candidates.clone();
        cx.spawn_in(window, async move |picker, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let executor = cx.background_executor().clone();
            let matches =
                fuzzy::match_strings_async(candidates, query, usize::MAX, &executor).await;
            picker
                .update(cx, |picker, _| {
                    picker.delegate.matches = matches;
                    picker.delegate.selected = 0;
                })
                .ok();
        })
    }

    fn confirm(&mut self, secondary: bool, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(text) = self.selected_text() {
            self.record.confirmed.borrow_mut().push((text, secondary));
            cx.emit(DismissEvent);
        }
    }

    fn dismissed(&mut self, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.record.dismissed.set(self.record.dismissed.get() + 1);
    }

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let found = self.matches.get(ix)?;
        self.record.rendered.set(self.record.rendered.get() + 1);
        let id = found.candidate_id;
        Some(
            match_label(found.string.clone(), &found.positions, selected, cx)
                .id(("test-item", id))
                .debug_selector(move || format!("test-item-{id}")),
        )
    }
}
