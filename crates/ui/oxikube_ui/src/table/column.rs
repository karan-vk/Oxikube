//! Column description and sort direction.

use gpui::{Pixels, SharedString, TextAlign, px};
use gpui_component::table::{Column, ColumnSort};

/// Sort state of a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortDirection {
    /// Not sorted by this column.
    #[default]
    Unsorted,
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

impl From<ColumnSort> for SortDirection {
    fn from(sort: ColumnSort) -> Self {
        match sort {
            ColumnSort::Default => SortDirection::Unsorted,
            ColumnSort::Ascending => SortDirection::Ascending,
            ColumnSort::Descending => SortDirection::Descending,
        }
    }
}

impl From<SortDirection> for ColumnSort {
    fn from(sort: SortDirection) -> Self {
        match sort {
            SortDirection::Unsorted => ColumnSort::Default,
            SortDirection::Ascending => ColumnSort::Ascending,
            SortDirection::Descending => ColumnSort::Descending,
        }
    }
}

/// Horizontal alignment of a column's header and cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlign {
    /// Left-aligned (text).
    #[default]
    Left,
    /// Centred.
    Center,
    /// Right-aligned (numbers, ages).
    Right,
}

/// One column of a [`Table`](super::Table).
///
/// Widths are pixels as given: pass them through [`crate::u`] to follow UI zoom.
#[derive(Clone, Debug)]
pub struct TableColumn {
    /// Stable identifier (usually the field name); survives reordering.
    pub key: SharedString,
    /// Header label.
    pub name: SharedString,
    /// Initial width.
    pub width: Pixels,
    /// Lower bound when the user resizes.
    pub min_width: Pixels,
    /// Header and cell alignment.
    pub align: ColumnAlign,
    /// `Some` makes the header clickable; the value is the current sort state.
    pub sort: Option<SortDirection>,
    /// Whether the user may drag the column edge.
    pub resizable: bool,
    /// Whether the user may drag the column to reorder it.
    pub movable: bool,
    /// Pin to the left edge while scrolling horizontally.
    pub fixed_left: bool,
}

impl TableColumn {
    /// A 100 px, left-aligned, unsortable column.
    pub fn new(key: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            width: px(100.),
            min_width: px(20.),
            align: ColumnAlign::Left,
            sort: None,
            resizable: true,
            movable: true,
            fixed_left: false,
        }
    }

    /// Initial width.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width.max(self.min_width);
        self
    }

    /// Minimum width (raises the width if it is below it).
    pub fn min_width(mut self, min_width: Pixels) -> Self {
        self.min_width = min_width;
        self.width = self.width.max(min_width);
        self
    }

    /// Right-align (numeric columns).
    pub fn right(mut self) -> Self {
        self.align = ColumnAlign::Right;
        self
    }

    /// Centre-align.
    pub fn center(mut self) -> Self {
        self.align = ColumnAlign::Center;
        self
    }

    /// Make the header sortable, currently unsorted.
    pub fn sortable(mut self) -> Self {
        self.sort = Some(SortDirection::Unsorted);
        self
    }

    /// Make the header sortable and mark `direction` as the active sort.
    pub fn sorted(mut self, direction: SortDirection) -> Self {
        self.sort = Some(direction);
        self
    }

    /// Allow or forbid resizing.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Allow or forbid reordering.
    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    /// Pin to the left edge.
    pub fn fixed_left(mut self) -> Self {
        self.fixed_left = true;
        self
    }

    /// The gpui-component column this describes.
    pub(super) fn to_library(&self) -> Column {
        let mut column = Column::new(self.key.clone(), self.name.clone())
            .width(self.width)
            .min_width(self.min_width)
            .resizable(self.resizable)
            .movable(self.movable);
        column.align = match self.align {
            ColumnAlign::Left => TextAlign::Left,
            ColumnAlign::Center => TextAlign::Center,
            ColumnAlign::Right => TextAlign::Right,
        };
        column.sort = self.sort.map(ColumnSort::from);
        if self.fixed_left {
            column = column.fixed_left();
        }
        column
    }
}
