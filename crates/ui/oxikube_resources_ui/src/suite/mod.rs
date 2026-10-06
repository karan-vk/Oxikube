//! The resource browser's end-to-end suite (E07-S12): the real table, filter bar, detail drawer
//! and row actions driven by scripted feeds, over testkit fakes. It is the epic's safety net: a
//! refactor of the table, the store façade or the detail that changes behaviour a user sees
//! fails here, whichever story owns the code.
//!
//! Every scenario goes through the same window as the app's: a workspace with cluster tabs, the
//! [`ResourceViews`](crate::ResourceViews) controller, a real [`ClusterSessionManager`] and
//! [`ResourceStores`], a real `CommandBus` and `MutationGuard` for the actions; only the ports
//! are fakes. Feeds are scripted with [`ScriptedFeed`] (add / modify / delete at chosen ticks,
//! delivered as the `DeltaBatch`es a watch carries) and time only moves when a test advances the
//! fake clock, so nothing sleeps and no thread starts.
//!
//! | File | Scenarios |
//! |---|---|
//! | `rows` | initial render of N rows, add / modify / delete while scrolled, sort stability across deltas, one redraw per delivered batch |
//! | `filter` | `/x`, `/!x`, `/-l`, `/-f` end to end, the filter surviving deltas |
//! | `selection` | selection and cursor survive deltas by object identity |
//! | `columns` | resize / reorder / hide / sort persisted per kind and restored by a recreated view |
//! | `detail` | Overview, YAML, Describe and Events tabs of a row opened from the table |
//! | `crd` | a custom resource table from a Table response (printer columns, live rows) |
//! | `delete` | delete through the bus and guard, and the read-only block |
//!
//! Which acceptance criteria of E07-S03..S08 each scenario covers is in the story's PR; the
//! visual baseline is `tests/screenshot.rs` (`pods_table_tones`, `pods_table_tones_light`).
//!
//! [`ClusterSessionManager`]: oxikube_app::ClusterSessionManager
//! [`ResourceStores`]: oxikube_app::store::ResourceStores

mod columns;
mod crd;
mod delete;
mod detail;
mod filter;
mod rows;
mod selection;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{Entity, Subscription, TestAppContext};
use oxikube_app::ColumnId;
use oxikube_domain::Resource;
use oxikube_testkit::{ScriptedFeed, TICK, pod};

use crate::table::ResourceTable;
use crate::table::tests::fixture::Fixture;

/// A pod `ns/name` with `restarts` restarts.
pub(super) fn pod_in(ns: &str, name: &str, restarts: u32) -> Resource {
    pod().namespace(ns).name(name).restarts(restarts).build()
}

/// The `pod-NNN` pods `range` of namespace `x`.
pub(super) fn numbered(range: std::ops::Range<u32>) -> Vec<Resource> {
    range
        .map(|i| pod_in("x", &format!("pod-{i:04}"), 0))
        .collect()
}

/// The display text of `column` in the row of `table` named `name`.
pub(super) fn cell(
    f: &mut Fixture,
    table: &Entity<ResourceTable>,
    name: &str,
    column: &str,
) -> String {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let row = d
                .rows()
                .iter()
                .find(|r| r.name() == name)
                .unwrap_or_else(|| panic!("{name} is not a row"));
            d.provider()
                .cell(row, &ColumnId::new(column), jiff::Timestamp::now())
                .display()
                .to_owned()
        })
    })
}

/// A pods table over a [`ScriptedFeed`]: tick 0 (the initial list) has arrived and
/// [`step`](Self::step) delivers the next tick.
pub(super) struct Scripted {
    pub(super) f: Fixture,
    pub(super) table: Entity<ResourceTable>,
    redraws: Rc<Cell<usize>>,
    tick: u32,
    last: u32,
    _observe: Subscription,
}

impl Scripted {
    /// Opens the pods table of a connected cluster whose pods feed is `feed`.
    pub(super) fn open(cx: &mut TestAppContext, feed: &ScriptedFeed) -> Self {
        Self::open_in(Fixture::new(cx), feed)
    }

    /// [`Self::open`] in an existing window (a "restarted app" shares its state port).
    pub(super) fn open_in(mut f: Fixture, feed: &ScriptedFeed) -> Self {
        feed.install(&f.ports().resources);
        f.connect_with([]);
        let table = f.open_pods();
        let redraws = Rc::new(Cell::new(0));
        let count = redraws.clone();
        let observe = f
            .vcx
            .update(|_, cx| cx.observe(&table, move |_, _| count.set(count.get() + 1)));
        Self {
            f,
            table,
            redraws,
            tick: 0,
            last: feed.last_tick(),
            _observe: observe,
        }
    }

    /// Delivers the next tick's batch and lets the store, the table and a coalesced redraw
    /// settle. Returns how many times the table asked to be redrawn meanwhile.
    pub(super) fn step(&mut self) -> usize {
        assert!(self.tick < self.last, "the feed has no more ticks");
        self.tick += 1;
        self.redraws.set(0);
        self.f.ports().resources.clock().advance(TICK);
        self.f.settle();
        self.redraws.get()
    }

    /// The row names, in order.
    pub(super) fn names(&mut self) -> Vec<String> {
        self.f.names(&self.table)
    }

    /// Draws a frame, so layout and the virtual list are current.
    pub(super) fn draw(&mut self) {
        self.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }
}
