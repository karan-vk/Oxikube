//! Column widths under UI zoom.
//!
//! gpui-component caches a column's width when it reads the column definitions (on creation and on
//! `refresh`) and then only changes it when the user drags. So the zoom has to be applied when the
//! definitions are read, and re-read when the zoom changes. [`ColumnWidths`] is the state shared by
//! the adapter (which builds the library columns), the handle (which records what the user
//! resized) and the element (which notices a zoom change and refreshes): widths a delegate gives
//! are design-time pixels, widths the user chose are kept [`Unscaled`], and both are multiplied by
//! the current zoom on read.

use crate::size::{UiScale, Unscaled};
use gpui::Pixels;
use std::cell::{Cell, RefCell};

/// Zoom-aware width state of one table.
pub(super) struct ColumnWidths {
    /// Widths the user dragged to, per column index. `None`: follow the delegate's width.
    overrides: RefCell<Vec<Option<Unscaled>>>,
    /// The on-screen widths handed to the library when it last read each column.
    supplied: RefCell<Vec<Pixels>>,
    /// The zoom the library's cached widths were computed at.
    applied_scale: Cell<f32>,
}

impl ColumnWidths {
    pub(super) fn new() -> Self {
        Self {
            overrides: RefCell::new(Vec::new()),
            supplied: RefCell::new(Vec::new()),
            applied_scale: Cell::new(crate::size::current_scale()),
        }
    }

    /// The width the user chose for `col_ix`, if they resized it.
    pub(super) fn user_width(&self, col_ix: usize) -> Option<Unscaled> {
        self.overrides.borrow().get(col_ix).copied().flatten()
    }

    /// Notes the on-screen width given to the library for `col_ix`.
    pub(super) fn record_supplied(&self, col_ix: usize, width: Pixels) {
        let mut supplied = self.supplied.borrow_mut();
        if supplied.len() <= col_ix {
            supplied.resize(col_ix + 1, Pixels::ZERO);
        }
        supplied[col_ix] = width;
    }

    /// The on-screen width the library has for `col_ix` (supplied or resized), once it has read
    /// the column.
    pub(super) fn supplied(&self, col_ix: usize) -> Option<Pixels> {
        self.supplied
            .borrow()
            .get(col_ix)
            .copied()
            .filter(|width| *width > Pixels::ZERO)
    }

    /// The user resized columns: `widths` are on-screen, in column order. Keeps (unscaled) the
    /// columns whose width differs from the last known on-screen width (what was supplied, or what
    /// the previous resize reported), then takes `widths` as the new baseline. Without that, a
    /// column dragged back to its supplied width would keep its earlier override.
    pub(super) fn record_resize(&self, widths: &[Pixels]) {
        let mut supplied = self.supplied.borrow_mut();
        let mut overrides = self.overrides.borrow_mut();
        let len = widths.len().max(overrides.len());
        overrides.resize(len, None);
        for (ix, &width) in widths.iter().enumerate() {
            let untouched = supplied
                .get(ix)
                .is_some_and(|s| (f32::from(*s) - f32::from(width)).abs() < 0.5);
            if !untouched {
                overrides[ix] = Some(Unscaled::from_scaled(width, self.applied_scale()));
            }
        }
        if supplied.len() < widths.len() {
            supplied.resize(widths.len(), Pixels::ZERO);
        }
        supplied[..widths.len()].copy_from_slice(widths);
    }

    /// On-screen `widths` (as the library reports them) as unscaled widths.
    pub(super) fn unscale(&self, widths: &[Pixels]) -> Vec<Unscaled> {
        let scale = self.applied_scale();
        widths
            .iter()
            .map(|&width| Unscaled::from_scaled(width, scale))
            .collect()
    }

    /// A column moved from `from` to `to`: its width state moves with it.
    pub(super) fn moved(&self, from: usize, to: usize) {
        let mut overrides = self.overrides.borrow_mut();
        if from < overrides.len() && to < overrides.len() {
            let width = overrides.remove(from);
            overrides.insert(to, width);
        }
        let mut supplied = self.supplied.borrow_mut();
        if from < supplied.len() && to < supplied.len() {
            let width = supplied.remove(from);
            supplied.insert(to, width);
        }
    }

    /// Forgets the widths the user chose (the delegate's columns changed).
    pub(super) fn clear_user_widths(&self) {
        self.overrides.borrow_mut().clear();
    }

    /// The zoom the library's cached widths are at.
    pub(super) fn applied_scale(&self) -> UiScale {
        UiScale::new(self.applied_scale.get())
    }

    /// Marks the library's widths as computed at the current zoom.
    pub(super) fn mark_applied(&self) {
        self.applied_scale.set(crate::size::current_scale());
    }

    /// Whether the zoom changed since the library read the columns.
    pub(super) fn is_stale(&self) -> bool {
        self.applied_scale.get() != crate::size::current_scale()
    }
}
