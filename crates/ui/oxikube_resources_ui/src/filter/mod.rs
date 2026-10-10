//! The filter bar of a resource table (E07-S04): `/text`, `/!text`, `/-l k=v`, `/-f fuzzy`.
//!
//! | File | Holds |
//! |---|---|
//! | `bar` | [`FilterBar`]: the text field, the parse error and the `123 of 4,812` count; [`FilterBarEvent`] |
//! | `chip` | the removable chip of the active filter and the error chip |
//! | `apply` | parsing each edit, the debounce (first keystroke at once, the rest once per frame, selectors after a pause) |
//! | `actions` | `resource_table::FocusFilter` and `ClearFilter` |
//! | `settings` | [`ResourceTableSettings`]: `resource_table.persist_filter` |
//! | `saved` | [`SavedFilter`]: the filter text of a kind in a cluster in the `StatePort`, while that setting is on (the default); clearing removes it |
//!
//! The grammar, the matching and the server-side selector live in
//! `oxikube_app::store::filter`; the bar only parses (to show errors) and tells the table the
//! result. Pressing `/` in a table focuses the bar and dispatches `table::FocusFilter`;
//! `escape` clears it and `enter` returns to the rows. While the field has the focus the table's
//! key context says `Editing` (read from the window's focus on every render, so from the first
//! key on and in an inactive window too), so bare keys (`a`, `s`, `j`, `k`, `/`) are text.

mod actions;
mod apply;
mod bar;
mod chip;
mod saved;
mod settings;

pub use actions::{ClearFilter, FocusFilter};
pub use bar::{DEBOUNCE, FilterBar, FilterBarEvent, SELECTOR_DEBOUNCE, thousands};
pub use saved::{FILTER_PREFIX, FILTER_VERSION, FilterWriter, SavedFilter, filter_key};
pub use settings::{ResourceTableContent, ResourceTableSettings};
