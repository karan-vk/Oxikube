//! The selector's GPUI actions and their key bindings.
//!
//! These are view-level actions (open, move, activate); what they change goes through the
//! `namespace::Select` and `namespace::ToggleFavourite` commands
//! ([`NamespaceService::execute`](oxikube_app::session::namespaces::NamespaceService::execute)).
//! The digit keys are one action, [`SelectSlot`], with the digit as data.

use gpui::{Action, App, KeyBinding};
use schemars::JsonSchema;
use serde::Deserialize;

/// Key context of the closed selector (its trigger).
pub const SELECTOR_CONTEXT: &str = "NamespaceSelector";
/// Key context added on the open list.
pub const LIST_CONTEXT: &str = "NamespaceList";
/// Key context of the search box's wrapper.
pub const SEARCH_CONTEXT: &str = "NamespaceSearch";

gpui::actions!(
    namespace_selector,
    [
        /// Opens the dropdown (on the trigger).
        Open,
        /// Closes the dropdown and returns focus to the trigger.
        Close,
        /// Highlights the previous row.
        MoveUp,
        /// Highlights the next row.
        MoveDown,
        /// Ticks or unticks the highlighted namespace, or picks "All namespaces".
        ToggleHighlighted,
        /// Pins or unpins the highlighted namespace.
        ToggleFavouriteHighlighted,
        /// Moves focus to the search box.
        FocusSearch,
        /// Moves focus from the search box to the list.
        FocusList,
    ]
);

/// Selects slot `slot`: `0` is all namespaces, `1`-`9` the favourite shown with that digit.
#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = namespace_selector)]
pub struct SelectSlot {
    /// The digit.
    pub slot: u8,
}

/// Binds the selector's keys. Called by [`super::register`].
pub(super) fn register(cx: &mut App) {
    let mut bindings = vec![
        KeyBinding::new("enter", Open, Some(SELECTOR_CONTEXT)),
        KeyBinding::new("down", Open, Some(SELECTOR_CONTEXT)),
        KeyBinding::new("escape", Close, Some(LIST_CONTEXT)),
        KeyBinding::new("up", MoveUp, Some(LIST_CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(LIST_CONTEXT)),
        KeyBinding::new("enter", ToggleHighlighted, Some(LIST_CONTEXT)),
        KeyBinding::new("space", ToggleHighlighted, Some(LIST_CONTEXT)),
        KeyBinding::new("f", ToggleFavouriteHighlighted, Some(LIST_CONTEXT)),
        KeyBinding::new("/", FocusSearch, Some(LIST_CONTEXT)),
        KeyBinding::new("escape", Close, Some(SEARCH_CONTEXT)),
        KeyBinding::new("down", FocusList, Some(SEARCH_CONTEXT)),
    ];
    // The digits work on the closed trigger and in the open list, but not in the search box,
    // where they are text.
    for slot in 0..=9u8 {
        for context in [SELECTOR_CONTEXT, LIST_CONTEXT] {
            bindings.push(KeyBinding::new(
                &slot.to_string(),
                SelectSlot { slot },
                Some(context),
            ));
        }
    }
    cx.bind_keys(bindings);
}
