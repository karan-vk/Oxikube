//! [`CellCache`]: the text and tone of the cells on screen, kept between frames (E07-S09).
//!
//! Reading a cell through the [`ColumnProvider`] is cheap but not free (a pod's `Ready`, `Status`
//! and `Restarts` cells each summarise the pod's JSON), and turning it into the text the table
//! draws allocates. While a table scrolls or churns, almost every visible cell is the same as in the
//! previous frame, so the cache keeps each visible row's cells, keyed by the row's object (a new
//! version of an object is a new `Arc`, so an update is a miss by construction) and column.
//!
//! Ages move with the clock: the whole cache is dropped when "now" crosses into a new second
//! (the table redraws ages once a second) and when the provider changes. Rows that were not drawn
//! in the previous frame are dropped at the start of the next one, so the cache holds about one
//! screen of rows; [`MAX_ROWS`] bounds it when the table scrolls without the view redrawing.

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

/// One row's cells, by column.
struct RowCells {
    /// The object the cells were read from (kept so its address cannot be reused meanwhile).
    _object: Arc<StoreObject>,
    cells: Vec<(ColumnId, SharedString, Tone)>,
    /// The frame that last drew this row.
    frame: u64,
}

/// See the [module docs](self).
#[derive(Default)]
pub struct CellCache {
    rows: HashMap<usize, RowCells>,
    /// The second of "now" the cells were read at.
    second: i64,
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
    /// Starts a frame drawn at `now` with `provider`: drops everything when the second or the
    /// provider changed, else the rows the previous frame did not draw.
    pub fn begin_frame(&mut self, now: Timestamp, provider: &Arc<dyn ColumnProvider>) {
        let second = now.as_second();
        let provider = address(provider);
        if second != self.second || provider != self.provider {
            self.rows.clear();
            self.second = second;
            self.provider = provider;
        } else {
            let last = self.frame;
            self.rows.retain(|_, row| row.frame == last);
        }
        self.frame += 1;
    }

    /// The text and tone of `object` in `column`, read through `provider` on a miss.
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
                _object: object.clone(),
                cells: Vec::new(),
                frame,
            });
        row.frame = frame;
        if let Some((_, text, tone)) = row.cells.iter().find(|(id, ..)| id == column) {
            return (text.clone(), *tone);
        }
        #[cfg(test)]
        {
            self.misses += 1;
        }
        let cell = provider.cell(object, column, now);
        let text = SharedString::from(cell.display().to_owned());
        let tone = cell.tone();
        row.cells.push((column.clone(), text.clone(), tone));
        (text, tone)
    }

    /// Whether any cell the last frame drew would read differently at `now`: an age that
    /// crossed into its next unit or second. The table's once-a-second tick asks this and
    /// redraws only on `true`, so a table that is on screen and still costs no frames (most ages
    /// are days old and change once a minute or less). Reads the cells of the drawn rows only,
    /// about one screen, never the whole list.
    pub fn ages_moved(&self, provider: &dyn ColumnProvider, now: Timestamp) -> bool {
        self.rows
            .values()
            .filter(|row| row.frame == self.frame)
            .any(|row| {
                row.cells.iter().any(|(column, text, tone)| {
                    let cell = provider.cell(&row._object, column, now);
                    cell.display() != text.as_ref() || cell.tone() != *tone
                })
            })
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

    #[test]
    fn hits_until_the_object_the_second_or_the_provider_changes() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let restarts = ColumnId::new("restarts");
        let mut cache = CellCache::default();
        let a = object("a", 3);

        // A restarts cell reads "3 (<age> ago)": text that moves with the clock.
        cache.begin_frame(now, &provider);
        assert!(cache.get(&a, &restarts, &*provider, now).0.starts_with('3'));
        assert!(cache.get(&a, &restarts, &*provider, now).0.starts_with('3'));
        assert_eq!(cache.misses, 1, "the second read is a hit");

        // The next frame, same second: still a hit.
        cache.begin_frame(now, &provider);
        cache.get(&a, &restarts, &*provider, now);
        assert_eq!(cache.misses, 1);

        // A new version of the object is a new Arc: a miss with the new value.
        let a2 = object("a", 4);
        assert!(
            cache
                .get(&a2, &restarts, &*provider, now)
                .0
                .starts_with('4')
        );
        assert_eq!(cache.misses, 2);

        // A new second re-reads (ages move).
        let later = now + jiff::SignedDuration::from_secs(1);
        cache.begin_frame(later, &provider);
        cache.get(&a2, &restarts, &*provider, later);
        assert_eq!(cache.misses, 3);

        // A new provider re-reads.
        let other: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        cache.begin_frame(later, &other);
        cache.get(&a2, &restarts, &*other, later);
        assert_eq!(cache.misses, 4);
    }

    #[test]
    fn ages_moved_only_when_a_drawn_cell_would_read_differently() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let age = ColumnId::new("age");
        let name = ColumnId::new(ColumnId::NAME);
        let mut cache = CellCache::default();
        // Created 30 days before `now`: its age string ("30d") moves once a day.
        let old = Arc::new(StoreObject::Resource(
            pod()
                .namespace("x")
                .name("old")
                .created((now - jiff::SignedDuration::from_hours(30 * 24)).to_string())
                .build(),
        ));
        let young = Arc::new(StoreObject::Resource(
            pod()
                .namespace("x")
                .name("young")
                .created((now - jiff::SignedDuration::from_secs(30)).to_string())
                .build(),
        ));
        let at = |s| now + jiff::SignedDuration::from_secs(s);

        // Nothing drawn yet: nothing can have moved.
        assert!(!cache.ages_moved(&*provider, at(3_600)));

        cache.begin_frame(now, &provider);
        cache.get(&old, &age, &*provider, now);
        cache.get(&old, &name, &*provider, now);
        assert!(!cache.ages_moved(&*provider, now));
        assert!(
            !cache.ages_moved(&*provider, at(1)),
            "30d is still 30d a second on"
        );
        assert!(!cache.ages_moved(&*provider, at(3_600)), "and an hour on");
        assert!(
            cache.ages_moved(&*provider, at(24 * 3_600)),
            "a day on it reads 31d"
        );

        // A row whose age is in seconds moves every second.
        cache.get(&young, &age, &*provider, now);
        assert!(cache.ages_moved(&*provider, at(1)));
        // But not while only its name is drawn.
        let mut names = CellCache::default();
        names.begin_frame(now, &provider);
        names.get(&young, &name, &*provider, now);
        assert!(!names.ages_moved(&*provider, at(1)));

        // Rows the last frame did not draw are not asked about.
        cache.begin_frame(now, &provider);
        cache.get(&old, &age, &*provider, now);
        assert!(!cache.ages_moved(&*provider, at(1)));
    }

    #[test]
    fn keeps_only_the_rows_the_previous_frame_drew() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let name = ColumnId::new(ColumnId::NAME);
        let mut cache = CellCache::default();
        let rows: Vec<_> = (0..10).map(|i| object(&format!("p{i}"), 0)).collect();

        cache.begin_frame(now, &provider);
        for row in &rows {
            cache.get(row, &name, &*provider, now);
        }
        assert_eq!(cache.len(), 10);
        // Scrolled: the next frame draws rows 5.. only.
        cache.begin_frame(now, &provider);
        for row in &rows[5..] {
            cache.get(row, &name, &*provider, now);
        }
        cache.begin_frame(now, &provider);
        assert_eq!(cache.len(), 5, "rows 0..5 left the screen");
    }

    #[test]
    fn is_bounded_while_scrolling_without_frames() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let name = ColumnId::new(ColumnId::NAME);
        let mut cache = CellCache::default();
        cache.begin_frame(now, &provider);
        for i in 0..(MAX_ROWS * 2 + 7) {
            cache.get(&object(&format!("p{i}"), 0), &name, &*provider, now);
        }
        assert!(cache.len() <= MAX_ROWS);
    }
}
