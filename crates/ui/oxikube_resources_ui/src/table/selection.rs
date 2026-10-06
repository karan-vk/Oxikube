//! [`Selection`]: which rows of a resource table are selected, by object identity.
//!
//! Rows move whenever the feed delivers (a pod restarts and its rank under the sort changes, a
//! new pod lands above the selected one), so the selection is kept by [`ObjectKey`] (namespace
//! and name; with the table's cluster and kind that is the row's `ResourceRef`), never by row
//! index. The cursor (the row the keyboard moves from and `enter` opens) and the anchor (where
//! a shift-click range starts) are keys too. Positions are looked up when needed, which is on a
//! key press, never per frame.

use std::collections::HashSet;
use std::sync::Arc;

use oxikube_app::store::{ObjectKey, RowOp, StoreObject};

/// How a click changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickMode {
    /// A plain click: select only this row.
    Replace,
    /// Cmd-click (ctrl elsewhere): add or remove this row.
    Toggle,
    /// Shift-click: select the range from the anchor to this row.
    Extend,
}

/// The selected rows of one table. See the [`table`](crate::table) module docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    selected: HashSet<ObjectKey>,
    anchor: Option<ObjectKey>,
    cursor: Option<ObjectKey>,
}

impl Selection {
    /// How many rows are selected.
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// Whether the row `key` is selected.
    pub fn contains(&self, key: &ObjectKey) -> bool {
        self.selected.contains(key)
    }

    /// Whether `object` is selected. Cheap: two reference-count bumps for the lookup key.
    pub fn contains_object(&self, object: &StoreObject) -> bool {
        !self.selected.is_empty() && self.selected.contains(&object.key())
    }

    /// The cursor row's key.
    pub fn cursor(&self) -> Option<&ObjectKey> {
        self.cursor.as_ref()
    }

    /// The selected keys, in no particular order.
    pub fn keys(&self) -> impl Iterator<Item = &ObjectKey> {
        self.selected.iter()
    }

    /// The selected keys in row order.
    pub fn in_row_order(&self, rows: &[Arc<StoreObject>]) -> Vec<ObjectKey> {
        rows.iter()
            .map(|row| row.key())
            .filter(|key| self.selected.contains(key))
            .collect()
    }

    /// Where the cursor row is now, if it is still listed.
    pub fn cursor_index(&self, rows: &[Arc<StoreObject>]) -> Option<usize> {
        let cursor = self.cursor.as_ref()?;
        position(rows, cursor)
    }

    /// The user clicked row `index` with `mode`.
    pub fn click(&mut self, rows: &[Arc<StoreObject>], index: usize, mode: ClickMode) {
        let Some(key) = rows.get(index).map(|row| row.key()) else {
            return;
        };
        match mode {
            ClickMode::Replace => self.only(key),
            ClickMode::Toggle => {
                if !self.selected.remove(&key) {
                    self.selected.insert(key.clone());
                }
                self.anchor = Some(key.clone());
                self.cursor = Some(key);
            }
            ClickMode::Extend => {
                let from = self
                    .anchor
                    .as_ref()
                    .and_then(|anchor| position(rows, anchor))
                    .unwrap_or(index);
                let (lo, hi) = (from.min(index), from.max(index));
                self.selected = rows[lo..=hi].iter().map(|row| row.key()).collect();
                if self.anchor.is_none() {
                    self.anchor = Some(key.clone());
                }
                self.cursor = Some(key);
            }
        }
    }

    /// Moves the cursor by `delta` rows (clamped to the list) and selects only that row; with
    /// `extend` the range from the anchor to the new cursor is selected instead. Without a
    /// cursor, moving down starts at the first row and moving up at the last. Returns the new
    /// cursor position.
    pub fn move_cursor(
        &mut self,
        rows: &[Arc<StoreObject>],
        delta: isize,
        extend: bool,
    ) -> Option<usize> {
        let last = rows.len().checked_sub(1)?;
        let target = match self.cursor_index(rows) {
            Some(current) => current.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        };
        self.go_to(rows, target, extend);
        Some(target)
    }

    /// Puts the cursor on row `index` (first / last row keys), selecting as
    /// [`move_cursor`](Self::move_cursor) does.
    pub fn go_to(&mut self, rows: &[Arc<StoreObject>], index: usize, extend: bool) {
        let mode = if extend {
            ClickMode::Extend
        } else {
            ClickMode::Replace
        };
        self.click(rows, index, mode);
    }

    /// Selects every row; the cursor stays where it was.
    pub fn select_all(&mut self, rows: &[Arc<StoreObject>]) {
        self.selected = rows.iter().map(|row| row.key()).collect();
    }

    /// Selects nothing and forgets the cursor.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The user right-clicked row `index`: a row outside the selection becomes the selection (the
    /// menu acts on what is selected); a row inside it keeps the selection.
    pub fn context_click(&mut self, rows: &[Arc<StoreObject>], index: usize) {
        let Some(key) = rows.get(index).map(|row| row.key()) else {
            return;
        };
        if self.selected.contains(&key) {
            self.cursor = Some(key);
        } else {
            self.only(key);
        }
    }

    /// Applies one feed delta to `rows` (the table's copy of the list) and drops from the
    /// selection the objects it deleted. A row that moved (removed and inserted again in one
    /// batch) stays selected.
    pub fn apply_ops(&mut self, rows: &mut Vec<Arc<StoreObject>>, ops: &[RowOp]) {
        if self.is_unset() {
            ops.iter().for_each(|op| op.apply(rows));
            return;
        }
        let mut removed = Vec::new();
        let mut inserted = HashSet::new();
        for op in ops {
            match op {
                RowOp::Remove { index } => {
                    if let Some(row) = rows.get(*index) {
                        removed.push(row.key());
                    }
                }
                RowOp::Insert { object, .. } => {
                    inserted.insert(object.key());
                }
                RowOp::Update { .. } => {}
            }
            op.apply(rows);
        }
        for key in removed.into_iter().filter(|key| !inserted.contains(key)) {
            self.forget(&key);
        }
    }

    /// Replaces `rows` with a snapshot and keeps only the selected objects it still lists.
    pub fn apply_snapshot(
        &mut self,
        rows: &mut Vec<Arc<StoreObject>>,
        snapshot: &[Arc<StoreObject>],
    ) {
        rows.clear();
        rows.extend_from_slice(snapshot);
        if self.is_unset() {
            return;
        }
        let listed: HashSet<ObjectKey> = snapshot
            .iter()
            .map(|row| row.key())
            .filter(|key| self.is_known(key))
            .collect();
        self.selected.retain(|key| listed.contains(key));
        if self.anchor.as_ref().is_some_and(|a| !listed.contains(a)) {
            self.anchor = None;
        }
        if self.cursor.as_ref().is_some_and(|c| !listed.contains(c)) {
            self.cursor = None;
        }
    }

    fn only(&mut self, key: ObjectKey) {
        self.selected.clear();
        self.selected.insert(key.clone());
        self.anchor = Some(key.clone());
        self.cursor = Some(key);
    }

    fn forget(&mut self, key: &ObjectKey) {
        self.selected.remove(key);
        if self.anchor.as_ref() == Some(key) {
            self.anchor = None;
        }
        if self.cursor.as_ref() == Some(key) {
            self.cursor = None;
        }
    }

    fn is_known(&self, key: &ObjectKey) -> bool {
        self.selected.contains(key)
            || self.anchor.as_ref() == Some(key)
            || self.cursor.as_ref() == Some(key)
    }

    fn is_unset(&self) -> bool {
        self.selected.is_empty() && self.anchor.is_none() && self.cursor.is_none()
    }
}

fn position(rows: &[Arc<StoreObject>], key: &ObjectKey) -> Option<usize> {
    rows.iter().position(|row| {
        let meta = row.meta();
        *meta.name == *key.name && meta.namespace == key.namespace
    })
}
