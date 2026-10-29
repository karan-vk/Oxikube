//! [`FilesModel`]: one directory's entries as the table shows them: sorted, filtered, with the
//! cell text made once.
//!
//! Plain data, no GPUI: the sort and the hidden-file filter are unit tested, including a
//! 10 000-entry directory.

use std::cmp::Ordering;
use std::sync::Arc;

use gpui::SharedString;
use oxikube_app::files::{FileEntry, FileKind, Listing, ListingSource};

use crate::files::format::{entry_size_text, mode_text, name_text, time_text};

/// A column of the file table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileColumn {
    /// The name, with the type icon.
    Name,
    /// The size.
    Size,
    /// The type letter and permission bits.
    Mode,
    /// The modification time.
    Modified,
}

impl FileColumn {
    /// Every column, in display order.
    pub const ALL: [FileColumn; 4] = [
        FileColumn::Name,
        FileColumn::Size,
        FileColumn::Mode,
        FileColumn::Modified,
    ];

    /// The column at `index`.
    pub fn at(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The header text.
    pub fn title(self) -> &'static str {
        match self {
            FileColumn::Name => "Name",
            FileColumn::Size => "Size",
            FileColumn::Mode => "Mode",
            FileColumn::Modified => "Modified",
        }
    }

    /// The key the table knows the column by.
    pub fn key(self) -> &'static str {
        match self {
            FileColumn::Name => "name",
            FileColumn::Size => "size",
            FileColumn::Mode => "mode",
            FileColumn::Modified => "modified",
        }
    }

    /// The default width in pixels.
    pub fn width(self) -> f32 {
        match self {
            FileColumn::Name => 360.0,
            FileColumn::Size => 90.0,
            FileColumn::Mode => 120.0,
            FileColumn::Modified => 175.0,
        }
    }
}

/// How the entries are ordered. Directories always come before files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSort {
    /// The column.
    pub column: FileColumn,
    /// Largest, latest or last-in-the-alphabet first.
    pub descending: bool,
}

impl Default for FileSort {
    fn default() -> Self {
        Self {
            column: FileColumn::Name,
            descending: false,
        }
    }
}

/// One entry and its cached cells.
#[derive(Debug, Clone)]
pub struct FileRow {
    /// The entry.
    pub entry: FileEntry,
    /// The name cell.
    pub name: SharedString,
    /// The size cell.
    pub size: SharedString,
    /// The mode cell.
    pub mode: SharedString,
    /// The time cell.
    pub modified: SharedString,
}

impl FileRow {
    fn new(entry: FileEntry) -> Self {
        Self {
            name: name_text(&entry).into(),
            size: entry_size_text(&entry).into(),
            mode: mode_text(&entry).into(),
            modified: entry.modified.map(time_text).unwrap_or_default().into(),
            entry,
        }
    }

    /// The cell of `column`.
    pub fn cell(&self, column: FileColumn) -> SharedString {
        match column {
            FileColumn::Name => self.name.clone(),
            FileColumn::Size => self.size.clone(),
            FileColumn::Mode => self.mode.clone(),
            FileColumn::Modified => self.modified.clone(),
        }
    }
}

/// A listing made ready to show: the cell text of every row and the order for one sort and
/// filter. Plain `Send` data, built where the listing was read (off the UI thread) so landing it
/// costs the UI thread a few pointer moves, not a pass over 10 000 entries; cheap to clone (the
/// rows are shared).
#[derive(Debug, Clone)]
pub struct PreparedListing {
    rows: Arc<Vec<FileRow>>,
    order: Arc<Vec<u32>>,
    sort: FileSort,
    show_hidden: bool,
    /// How many entries `ls` refused to describe.
    pub unreadable: usize,
    /// How the listing was made.
    pub source: ListingSource,
}

impl PreparedListing {
    /// Makes the rows of `listing`, ordered by `sort` with dot files shown or not.
    pub fn new(listing: &Listing, sort: FileSort, show_hidden: bool) -> Self {
        let rows: Vec<FileRow> = listing.entries.iter().cloned().map(FileRow::new).collect();
        let order = order_of(&rows, sort, show_hidden);
        Self {
            rows: Arc::new(rows),
            order: Arc::new(order),
            sort,
            show_hidden,
            unreadable: listing.unreadable,
            source: listing.source,
        }
    }
}

fn order_of(rows: &[FileRow], sort: FileSort, show_hidden: bool) -> Vec<u32> {
    let mut order: Vec<u32> = (0..rows.len())
        .filter(|&i| show_hidden || !rows[i].entry.is_hidden())
        .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect();
    order.sort_by(|&a, &b| compare(&rows[a as usize].entry, &rows[b as usize].entry, sort));
    order
}

/// The entries of the directory the view shows.
#[derive(Debug, Default)]
pub struct FilesModel {
    rows: Arc<Vec<FileRow>>,
    /// Indices into `rows` of the rows shown, in order.
    order: Arc<Vec<u32>>,
    sort: FileSort,
    show_hidden: bool,
    /// The entry selected, by name (it follows the entry through a re-sort or a refresh).
    selected: Option<String>,
    /// The row it is shown at: kept in step with `order`, so asking is not a scan.
    selected_row: Option<usize>,
}

impl FilesModel {
    /// An empty model: dot files hidden, sorted by name.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the entries with `listing`'s and keeps the selection when the entry is still
    /// there. Makes the cells here: the view uses [`PreparedListing`] off the UI thread instead.
    pub fn set_listing(&mut self, listing: &Listing) {
        let prepared = PreparedListing::new(listing, self.sort, self.show_hidden);
        self.install(&prepared);
    }

    /// Shows a prepared listing, keeping the selection when the entry is still there. Reuses the
    /// prepared order when it was made for the sort and filter in force; else sorts again.
    pub fn install(&mut self, prepared: &PreparedListing) {
        self.rows = prepared.rows.clone();
        if prepared.sort == self.sort && prepared.show_hidden == self.show_hidden {
            self.order = prepared.order.clone();
        } else {
            self.rebuild();
        }
        self.resync_selection();
    }

    /// Shows no entries.
    pub fn clear(&mut self) {
        self.rows = Arc::default();
        self.order = Arc::default();
        self.selected = None;
        self.selected_row = None;
    }

    /// The order in force.
    pub fn sort(&self) -> FileSort {
        self.sort
    }

    /// Sets the order and re-sorts.
    pub fn set_sort(&mut self, sort: FileSort) {
        self.sort = sort;
        self.rebuild();
    }

    /// Whether dot files are shown.
    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Shows or hides dot files.
    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
        self.rebuild();
    }

    /// How many rows are shown.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether no row is shown.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// How many entries the directory has, hidden ones included.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// How many entries are hidden by the dot-file filter.
    pub fn hidden(&self) -> usize {
        self.rows.len() - self.order.len()
    }

    /// The row shown at `index`.
    pub fn row(&self, index: usize) -> Option<&FileRow> {
        self.order.get(index).map(|&i| &self.rows[i as usize])
    }

    /// The rows shown, in order.
    pub fn rows(&self) -> impl Iterator<Item = &FileRow> {
        self.order.iter().map(|&i| &self.rows[i as usize])
    }

    /// Selects the row shown at `index`.
    pub fn select(&mut self, index: usize) {
        self.selected = self.row(index).map(|row| row.entry.name.clone());
        self.selected_row = self.selected.as_ref().map(|_| index);
    }

    /// Clears the selection.
    pub fn deselect(&mut self) {
        self.selected = None;
        self.selected_row = None;
    }

    /// The entry selected.
    pub fn selected(&self) -> Option<&FileEntry> {
        self.row(self.selected_row?).map(|row| &row.entry)
    }

    /// The row index shown for the entry selected.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_row
    }

    /// Finds the selected entry again after the rows or their order changed; forgets it when it
    /// is gone.
    fn resync_selection(&mut self) {
        self.selected_row = self.selected.as_deref().and_then(|name| {
            self.order
                .iter()
                .position(|&i| self.rows[i as usize].entry.name == name)
        });
        if self.selected_row.is_none() {
            self.selected = None;
        }
    }

    fn rebuild(&mut self) {
        self.order = Arc::new(order_of(&self.rows, self.sort, self.show_hidden));
        self.resync_selection();
    }
}

fn is_directory_like(kind: FileKind) -> bool {
    kind == FileKind::Directory
}

fn compare(a: &FileEntry, b: &FileEntry, sort: FileSort) -> Ordering {
    // Directories first, whatever the column or direction.
    let by_kind = is_directory_like(b.kind).cmp(&is_directory_like(a.kind));
    if by_kind != Ordering::Equal {
        return by_kind;
    }
    let by_name = || natural_cmp(&a.name, &b.name);
    let ordering = match sort.column {
        FileColumn::Name => by_name(),
        FileColumn::Size => a.size.cmp(&b.size).then_with(by_name),
        FileColumn::Mode => a.mode.cmp(&b.mode).then_with(by_name),
        FileColumn::Modified => a.modified.cmp(&b.modified).then_with(by_name),
    };
    if sort.descending {
        ordering.reverse()
    } else {
        ordering
    }
}

/// Case-insensitive, with runs of digits compared as numbers (`file2` before `file10`).
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (nx, ny) = (take_number(&mut a), take_number(&mut b));
                let ordering = nx.0.cmp(&ny.0).then_with(|| nx.1.cmp(&ny.1));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (Some(x), Some(y)) => {
                let ordering = x.to_lowercase().cmp(y.to_lowercase());
                if ordering != Ordering::Equal {
                    return ordering;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// A run of digits as (digits without leading zeros, original length).
fn take_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> (String, usize) {
    let mut digits = String::new();
    let mut length = 0;
    while let Some(&c) = chars.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        length += 1;
        if !(digits.is_empty() && c == '0') {
            digits.push(c);
        }
        chars.next();
    }
    // A longer number is a bigger one.
    (format!("{:08}{digits}", digits.len()), length)
}
