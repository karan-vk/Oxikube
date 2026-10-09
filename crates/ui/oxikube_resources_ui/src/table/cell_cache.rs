//! [`CellCache`]: the text and tone of the cells on screen, kept between frames (E07-S09).
//!
//! Reading a cell through the [`ColumnProvider`] is cheap but not free (a pod's `Ready`, `Status`
//! and `Restarts` cells each summarise the pod's JSON), and turning it into the text the table
//! draws allocates. While a table scrolls or churns, almost every visible cell is the same as in the
//! previous frame, so the cache keeps each visible row's cells, keyed by the row's object (a new
//! version of an object is a new `Arc`, so an update is a miss by construction) and column.
//!
//! Ages move with the clock. A cell that does ([`Cell::moves_in`](oxikube_app::Cell::moves_in):
//! an age, `3 (5m ago)`) is kept with the moment its text changes and read again from then on;
//! every other cell is kept for as long as its row is drawn (#605: before, the whole cache was
//! dropped every second, so the frame that moved one age re-read every cell on screen).
//! [`CellCache::next_move`] is when the first drawn cell moves, which the table's age tick sleeps
//! until, and [`CellCache::refresh_moved`] re-reads the cells due then and says whether one reads
//! differently. The cache is dropped when the provider changes. Rows that were not drawn in the
//! previous frame are dropped at the start of the next one, so the cache holds about one screen of
//! rows; [`MAX_ROWS`] bounds it when the table scrolls without the view redrawing.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::SharedString;
use jiff::Timestamp;
use oxikube_app::ColumnId;
use oxikube_app::ColumnProvider;
use oxikube_app::columns::Tone;
use oxikube_app::store::StoreObject;

/// Rows kept at most (a few screens); beyond it the cache starts over.
const MAX_ROWS: usize = 1024;

/// One cell as last read.
struct CachedCell {
    column: ColumnId,
    text: SharedString,
    tone: Tone,
    /// When its text changes on its own (`None`: it does not move with the clock).
    moves_at: Option<Timestamp>,
}

impl CachedCell {
    fn read(
        object: &StoreObject,
        column: &ColumnId,
        provider: &dyn ColumnProvider,
        now: Timestamp,
    ) -> Self {
        let cell = provider.cell(object, column, now);
        Self {
            column: column.clone(),
            text: SharedString::from(cell.display().to_owned()),
            tone: cell.tone(),
            moves_at: cell
                .moves_in()
                .and_then(|after| now.checked_add(after).ok()),
        }
    }

    /// Whether it is still what it reads at `now`.
    fn current(&self, now: Timestamp) -> bool {
        self.moves_at.is_none_or(|at| at > now)
    }
}

/// One row's cells, by column.
struct RowCells {
    /// The object the cells were read from (kept so its address cannot be reused meanwhile).
    object: Arc<StoreObject>,
    cells: Vec<CachedCell>,
    /// The frame that last drew this row.
    frame: u64,
}

/// See the [module docs](self).
#[derive(Default)]
pub struct CellCache {
    rows: HashMap<usize, RowCells>,
    /// The provider the cells were read from (its address).
    provider: usize,
    frame: u64,
    #[cfg(test)]
    pub(super) misses: usize,
}

fn address<T: ?Sized>(arc: &Arc<T>) -> usize {
    Arc::as_ptr(arc).cast::<()>() as usize
}

impl CellCache {
    /// Starts a frame drawn with `provider`: drops everything when the provider changed, else the
    /// rows the previous frame did not draw.
    pub fn begin_frame(&mut self, provider: &Arc<dyn ColumnProvider>) {
        let provider = address(provider);
        if provider != self.provider {
            self.rows.clear();
            self.provider = provider;
        } else {
            let last = self.frame;
            self.rows.retain(|_, row| row.frame == last);
        }
        self.frame += 1;
    }

    /// The text and tone of `object` in `column` at `now`, read through `provider` on a miss (a
    /// cell not read yet, or one whose text has moved since).
    pub fn get(
        &mut self,
        object: &Arc<StoreObject>,
        column: &ColumnId,
        provider: &dyn ColumnProvider,
        now: Timestamp,
    ) -> (SharedString, Tone) {
        if self.rows.len() >= MAX_ROWS {
            self.rows.clear();
        }
        let frame = self.frame;
        let row = self
            .rows
            .entry(address(object))
            .or_insert_with(|| RowCells {
                object: object.clone(),
                cells: Vec::new(),
                frame,
            });
        row.frame = frame;
        let at = row.cells.iter().position(|c| c.column == *column);
        if let Some(cell) = at.map(|i| &row.cells[i])
            && cell.current(now)
        {
            return (cell.text.clone(), cell.tone);
        }
        #[cfg(test)]
        {
            self.misses += 1;
        }
        let cell = CachedCell::read(object, column, provider, now);
        let out = (cell.text.clone(), cell.tone);
        match at {
            Some(i) => row.cells[i] = cell,
            None => row.cells.push(cell),
        }
        out
    }

    /// When the first cell the last frame drew moves (see the [module docs](self)); `None` when
    /// none moves with the clock. About one screen of cells, no provider reads.
    pub fn next_move(&self) -> Option<Timestamp> {
        self.drawn()
            .flat_map(|row| row.cells.iter().filter_map(|c| c.moves_at))
            .min()
    }

    /// Re-reads the drawn cells that have moved by `now` and keeps their new text, so the frame
    /// that shows them reads nothing else. Whether one of them now reads (or is coloured)
    /// differently: the table redraws only then.
    pub fn refresh_moved(&mut self, provider: &dyn ColumnProvider, now: Timestamp) -> bool {
        let frame = self.frame;
        let mut changed = false;
        for row in self.rows.values_mut().filter(|row| row.frame == frame) {
            for cell in row.cells.iter_mut().filter(|c| !c.current(now)) {
                let fresh = CachedCell::read(&row.object, &cell.column, provider, now);
                changed |= fresh.text != cell.text || fresh.tone != cell.tone;
                *cell = fresh;
            }
        }
        changed
    }

    fn drawn(&self) -> impl Iterator<Item = &RowCells> {
        self.rows
            .values()
            .filter(move |row| row.frame == self.frame)
    }

    /// Rows held now.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_app::CoreColumns;
    use oxikube_testkit::pod;

    fn object(name: &str, restarts: u32) -> Arc<StoreObject> {
        Arc::new(StoreObject::Resource(
            pod().namespace("x").name(name).restarts(restarts).build(),
        ))
    }

    fn aged(name: &str, now: Timestamp, age: jiff::SignedDuration) -> Arc<StoreObject> {
        Arc::new(StoreObject::Resource(
            pod()
                .namespace("x")
                .name(name)
                .created((now - age).to_string())
                .build(),
        ))
    }

    #[test]
    fn hits_until_the_object_or_the_provider_changes_or_the_text_moves() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let restarts = ColumnId::new("restarts");
        let name = ColumnId::new(ColumnId::NAME);
        let mut cache = CellCache::default();
        let a = object("a", 3);

        // A restarts cell reads "3 (<age> ago)": text that moves with the clock.
        cache.begin_frame(&provider);
        assert!(cache.get(&a, &restarts, &*provider, now).0.starts_with('3'));
        assert!(cache.get(&a, &restarts, &*provider, now).0.starts_with('3'));
        cache.get(&a, &name, &*provider, now);
        assert_eq!(cache.misses, 2, "the second read is a hit");

        // The next frame, same second: still hits.
        cache.begin_frame(&provider);
        cache.get(&a, &restarts, &*provider, now);
        cache.get(&a, &name, &*provider, now);
        assert_eq!(cache.misses, 2);

        // A new version of the object is a new Arc: a miss with the new value.
        let a2 = object("a", 4);
        assert!(
            cache
                .get(&a2, &restarts, &*provider, now)
                .0
                .starts_with('4')
        );
        cache.get(&a2, &name, &*provider, now);
        assert_eq!(cache.misses, 4);

        // A second on, the name has not moved: a hit (#605: the cache is no longer dropped every
        // second). A day on, the restart age has moved: that one cell is read again.
        let later = now + jiff::SignedDuration::from_secs(1);
        cache.begin_frame(&provider);
        cache.get(&a2, &name, &*provider, later);
        assert_eq!(cache.misses, 4);
        let tomorrow = now + jiff::SignedDuration::from_hours(24);
        cache.get(&a2, &restarts, &*provider, tomorrow);
        cache.get(&a2, &name, &*provider, tomorrow);
        assert_eq!(cache.misses, 5);

        // A new provider re-reads.
        let other: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        cache.begin_frame(&other);
        cache.get(&a2, &name, &*other, tomorrow);
        assert_eq!(cache.misses, 6);
    }

    #[test]
    fn the_next_move_is_the_first_drawn_cell_to_read_differently() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let age = ColumnId::new("age");
        let name = ColumnId::new(ColumnId::NAME);
        let at = |s| now + jiff::SignedDuration::from_secs(s);
        // "30d" moves once a day; "30s" every second.
        let old = aged("old", now, jiff::SignedDuration::from_hours(30 * 24));
        let young = aged("young", now, jiff::SignedDuration::from_secs(30));
        let mut cache = CellCache::default();

        // Nothing drawn yet: nothing moves.
        assert_eq!(cache.next_move(), None);
        cache.begin_frame(&provider);
        cache.get(&old, &name, &*provider, now);
        assert_eq!(cache.next_move(), None, "a name never moves");
        cache.get(&old, &age, &*provider, now);
        assert_eq!(cache.next_move(), Some(at(24 * 3_600)));
        assert!(
            !cache.refresh_moved(&*provider, at(3_600)),
            "30d an hour on"
        );
        assert!(
            cache.refresh_moved(&*provider, at(24 * 3_600)),
            "31d a day on"
        );
        assert_eq!(cache.get(&old, &age, &*provider, at(24 * 3_600)).0, "31d");
        assert_eq!(cache.next_move(), Some(at(2 * 24 * 3_600)));

        // A row whose age is in seconds moves every second.
        cache.get(&young, &age, &*provider, now);
        assert_eq!(cache.next_move(), Some(at(1)));
        let misses = cache.misses;
        assert!(cache.refresh_moved(&*provider, at(1)));
        assert_eq!(cache.misses, misses, "refreshing is not a miss");
        assert_eq!(cache.get(&young, &age, &*provider, at(1)).0, "31s");
        assert_eq!(cache.misses, misses, "the frame reads the refreshed text");
        assert_eq!(cache.next_move(), Some(at(2)));

        // Rows the last frame did not draw are not asked about.
        cache.begin_frame(&provider);
        cache.get(&old, &age, &*provider, at(1));
        assert_eq!(cache.next_move(), Some(at(2 * 24 * 3_600)));
        assert!(!cache.refresh_moved(&*provider, at(5)));
    }

    #[test]
    fn keeps_only_the_rows_the_previous_frame_drew() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let name = ColumnId::new(ColumnId::NAME);
        let mut cache = CellCache::default();
        let rows: Vec<_> = (0..10).map(|i| object(&format!("p{i}"), 0)).collect();

        cache.begin_frame(&provider);
        for row in &rows {
            cache.get(row, &name, &*provider, now);
        }
        assert_eq!(cache.len(), 10);
        // Scrolled: the next frame draws rows 5.. only.
        cache.begin_frame(&provider);
        for row in &rows[5..] {
            cache.get(row, &name, &*provider, now);
        }
        cache.begin_frame(&provider);
        assert_eq!(cache.len(), 5, "rows 0..5 left the screen");
    }

    #[test]
    fn is_bounded_while_scrolling_without_frames() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let name = ColumnId::new(ColumnId::NAME);
        let mut cache = CellCache::default();
        cache.begin_frame(&provider);
        for i in 0..(MAX_ROWS * 2 + 7) {
            cache.get(&object(&format!("p{i}"), 0), &name, &*provider, now);
        }
        assert!(cache.len() <= MAX_ROWS);
    }
}
