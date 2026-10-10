//! The namespace selector (E06-S07): a dropdown in the cluster tab's toolbar.
//!
//! * **What it shows.** The trigger reads `All namespaces`, `prod` or `prod +2`. The dropdown
//!   lists `All namespaces` (digit `0`), the favourites (digits `1`-`9`, shown next to each),
//!   then every other namespace, each with a tick, a star and its digit. A search box at the top
//!   filters the list locally as you type; the list is virtualised.
//! * **Selecting.** Click or Enter/Space on a namespace ticks it, so several can be selected;
//!   unticking the last one gives All (an empty selection is All, decided in the domain and
//!   pinned by the tests). Rapid ticks are debounced 150 ms into one change of the session's
//!   selection, so five clicks re-scope the feeds once.
//! * **Keys.** `0`-`9` select All or the nth favourite, on the focused trigger and in the open
//!   list (k9s). In the list: Up/Down move, Enter or Space ticks, `f` pins, `/` searches, Escape
//!   closes. In the search box: type to filter, Down goes to the list, Enter ticks the first
//!   match, Escape closes. The digits do not fire in the search box, where they are text.
//! * **Commands.** Every change is a [`Command`](oxikube_domain::command::Command) run by the
//!   [`NamespaceService`](oxikube_app::session::namespaces::NamespaceService): `namespace::Select`
//!   (digits, All, and the debounced ticks) and `namespace::ToggleFavourite` (the star and `f`).
//!   The GPUI actions here (`namespace_selector::*`) are only the view's own movements and the
//!   digit slot.
//! * **Restricted clusters.** When the cluster answers `403` to the namespace list, the dropdown
//!   says so and offers the cluster's `accessible_namespaces` setting and the names the user
//!   typed; typing a valid name offers `Add "name"`. The typed names are remembered per cluster.
//! * **Stale names.** A remembered namespace that no longer exists is dropped when the selector
//!   opens, and [`NamespaceSelectorEvent::StaleDropped`] asks the host for a toast
//!   ([`stale_dropped_toast`]).
//!
//! Nothing here touches the cluster or opens a feed: the service does its work through the
//! runtime bridge off the UI thread, and the `ResourceStore` re-scopes from the session's
//! `NamespaceChanged`. The view updates itself first, so input costs one frame. A digit or All
//! sets the session's selection in the same update (`NamespaceService::select_now`) and echoes it
//! to the views (`SessionEcho`), so the table narrows in that frame too (E05-P600).

mod actions;
mod background;
mod events;
mod model;
mod render;
mod selector;

#[cfg(test)]
mod tests;

pub use actions::{
    Close, FocusList, FocusSearch, LIST_CONTEXT, MoveDown, MoveUp, Open, SEARCH_CONTEXT,
    SELECTOR_CONTEXT, SelectSlot, ToggleFavouriteHighlighted, ToggleHighlighted,
};
pub use events::{NamespaceSelectorEvent, stale_dropped_toast};
pub use model::{NamespaceRow, Row, build_rows, selection_label};
pub use selector::NamespaceSelector;

/// Registers the selector's key bindings. Called by [`crate::init`].
pub fn register(cx: &mut gpui::App) {
    actions::register(cx);
}
