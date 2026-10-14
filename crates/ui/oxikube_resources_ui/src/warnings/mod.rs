//! Warning badges in the tables (E14-S04): a narrow icon column on every resource table that
//! shows when the row's object had a `Warning` event within the window (`events.warning_window`,
//! default one hour), a tooltip with the latest warning, and a click that opens the object's
//! Events tab (`resource::ViewEvents`).
//!
//! | File | Holds |
//! |---|---|
//! | `handle` | [`ObjectEvents`]: the app's [`WarningBadges`] as a GPUI global, installed by the binary |
//! | `column` | [`badge_column`], the column's id and its position; [`RowWarnings`], a table's source of badges |
//! | `cell` | the icon cell, its tooltip text and the click |
//! | `follow` | what keeps a table current: redraw when the badges change |
//!
//! # Performance
//!
//! A frame reads one hash entry per visible row ([`oxikube_app::ClusterBadges::get`], a shared
//! lock and a lookup by the object's uid): no event is scanned and nothing is allocated for a row
//! without a badge. The badge column is not sortable and its cell is never cached, so it adds no
//! work to the store's ranking or the cell cache. The model changes off the UI thread (the
//! tracker task of [`oxikube_app::BadgeTrack`]); a table is told by a coalesced stream and
//! redraws through its render gate, and the table's once-a-second tick notices an expired badge.
//!
//! Without the global (a window with no events service) the column is not offered.

mod cell;
mod column;
mod follow;
mod handle;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use cell::{badge_cell, cell_text};
pub(crate) use column::RowWarnings;
pub use column::{BADGE_COLUMN, badge_column};
pub(crate) use follow::follow_badges;
pub use handle::ObjectEvents;
