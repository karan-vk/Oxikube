//! The selection of [`EventsModel`]: one event, kept by its key, so rows arriving above it never
//! move it to another event.

use super::{EventRow, EventsModel};

impl EventsModel {
    /// The selected event's row, if it has one.
    pub fn selected_index(&self) -> Option<usize> {
        let key = self.selected.as_ref()?;
        let (id, event) = self.set.by_key.get(key)?;
        self.position(*id, event)
    }

    /// The selected row.
    pub fn selected_row(&self) -> Option<&EventRow> {
        self.selected_index().map(|ix| &*self.set.rows[ix])
    }

    /// Whether row `ix` is the selected one: a key comparison, for every visible row each frame.
    pub fn is_selected(&self, ix: usize) -> bool {
        match (&self.selected, self.set.rows.get(ix)) {
            (Some(key), Some(row)) => row.key == *key,
            _ => false,
        }
    }

    /// Selects row `ix` (nothing past the end). The selection follows the event, not the row
    /// number, so rows arriving above it do not move it to another event.
    pub fn select(&mut self, ix: usize) -> bool {
        let Some(row) = self.set.rows.get(ix) else {
            return false;
        };
        let changed = self.selected.as_ref() != Some(&row.key);
        self.selected = Some(row.key.clone());
        changed
    }

    /// Moves the selection `delta` rows (the first row when nothing is selected); returns the
    /// row selected.
    pub fn step(&mut self, delta: isize) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        let last = self.len() - 1;
        let ix = match self.selected_index() {
            Some(at) => at.saturating_add_signed(delta).min(last),
            None => 0,
        };
        self.select(ix);
        Some(ix)
    }

    /// Selects the first (`false`) or last (`true`) row.
    pub fn select_end(&mut self, last: bool) -> Option<usize> {
        let ix = if last {
            self.len().checked_sub(1)?
        } else if self.is_empty() {
            return None;
        } else {
            0
        };
        self.select(ix);
        Some(ix)
    }
}
