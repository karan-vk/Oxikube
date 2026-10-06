//! The `0`-`9` shortcuts (k9s): `0` is "all namespaces", `1`-`9` are the first nine favourites.

use oxikube_domain::session::{NamespaceFavourites, NamespaceSelection};

/// How many favourites have a digit: `1` to `9`.
pub const MAX_SLOTS: usize = 9;

/// The selection digit `slot` stands for: `0` is [`NamespaceSelection::All`], `1`-`9` the
/// matching favourite alone. `None` when `slot` is above 9 or no favourite sits there.
pub fn slot_selection(favourites: &NamespaceFavourites, slot: u8) -> Option<NamespaceSelection> {
    match slot {
        0 => Some(NamespaceSelection::All),
        1..=9 => favourites
            .iter()
            .nth(usize::from(slot) - 1)
            .map(NamespaceSelection::single),
        _ => None,
    }
}

/// The digit that selects `namespace` (`1`-`9`), when it is one of the first nine favourites.
/// The selector shows it next to the favourite.
pub fn favourite_slot(favourites: &NamespaceFavourites, namespace: &str) -> Option<u8> {
    favourites
        .iter()
        .take(MAX_SLOTS)
        .position(|n| n == namespace)
        .and_then(|i| u8::try_from(i + 1).ok())
}
