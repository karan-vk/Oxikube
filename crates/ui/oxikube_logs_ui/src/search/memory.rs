//! [`SearchMemory`]: the searches of the log views a window closed, kept for the session.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use oxikube_domain::ids::ResourceRef;

use super::state::SavedSearch;

/// The search of each log target, remembered while the app runs so that closing a log tab and
/// opening the same pod's logs again in the same session finds the filter as it was (E08-S03).
/// Nothing is written to disk: a restart starts clean. Cheap to clone; every view of a window
/// shares one.
#[derive(Clone, Default)]
pub struct SearchMemory {
    searches: Rc<RefCell<HashMap<ResourceRef, SavedSearch>>>,
}

impl SearchMemory {
    /// An empty memory.
    pub fn new() -> Self {
        Self::default()
    }

    /// The search remembered for `target`.
    pub fn get(&self, target: &ResourceRef) -> Option<SavedSearch> {
        self.searches.borrow().get(target).cloned()
    }

    /// Remembers `search` for `target`.
    pub fn save(&self, target: &ResourceRef, search: SavedSearch) {
        self.searches.borrow_mut().insert(target.clone(), search);
    }

    /// Forgets `target`'s search (its bar was closed).
    pub fn forget(&self, target: &ResourceRef) {
        self.searches.borrow_mut().remove(target);
    }

    /// Targets with a remembered search.
    pub fn len(&self) -> usize {
        self.searches.borrow().len()
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.searches.borrow().is_empty()
    }
}
