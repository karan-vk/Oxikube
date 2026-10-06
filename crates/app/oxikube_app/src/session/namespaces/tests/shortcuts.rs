//! The `0`-`9` mapping.

use oxikube_domain::session::{NamespaceFavourites, NamespaceSelection};
use proptest::prelude::*;

use crate::session::namespaces::{MAX_SLOTS, favourite_slot, slot_selection};

fn favourites(names: &[&str]) -> NamespaceFavourites {
    names.iter().copied().collect()
}

#[test]
fn zero_is_all_and_the_digits_are_the_favourites_in_order() {
    let favs = favourites(&["prod", "dev", "stage"]);

    assert_eq!(slot_selection(&favs, 0), Some(NamespaceSelection::All));
    assert_eq!(
        slot_selection(&favs, 1),
        Some(NamespaceSelection::single("prod"))
    );
    assert_eq!(
        slot_selection(&favs, 3),
        Some(NamespaceSelection::single("stage"))
    );
    assert_eq!(slot_selection(&favs, 4), None, "no fourth favourite");
    assert_eq!(slot_selection(&favs, 10), None, "not a digit");
    assert_eq!(
        slot_selection(&NamespaceFavourites::new(), 0),
        Some(NamespaceSelection::All),
        "0 needs no favourites"
    );
}

#[test]
fn only_the_first_nine_favourites_have_a_digit() {
    let names: Vec<String> = (1..=12).map(|i| format!("ns{i}")).collect();
    let favs: NamespaceFavourites = names.iter().map(String::as_str).collect();

    assert_eq!(favourite_slot(&favs, "ns1"), Some(1));
    assert_eq!(favourite_slot(&favs, "ns9"), Some(9));
    assert_eq!(favourite_slot(&favs, "ns10"), None);
    assert_eq!(favourite_slot(&favs, "other"), None);
    assert_eq!(
        slot_selection(&favs, 9),
        Some(NamespaceSelection::single("ns9"))
    );
}

proptest! {
    #[test]
    fn a_slot_and_its_favourite_agree(names in proptest::collection::vec("[a-z][a-z0-9-]{0,8}", 0..14)) {
        let favs: NamespaceFavourites = names.iter().map(String::as_str).collect();
        for slot in 1..=MAX_SLOTS as u8 {
            match slot_selection(&favs, slot) {
                Some(NamespaceSelection::Set(set)) => {
                    let name = set.iter().next().unwrap();
                    prop_assert_eq!(favourite_slot(&favs, name), Some(slot));
                }
                None => prop_assert!(favs.len() < usize::from(slot)),
                Some(NamespaceSelection::All) => prop_assert!(false, "digits 1-9 are never All"),
            }
        }
    }
}
