//! [`FindPane`]: what the detail keeps of its find.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Entity, Subscription, Task};
use oxikube_app::search::filter::FilterError;
use oxikube_app::search::find::{FindNavigator, FindQuery};
use oxikube_ui::input::InputState;

/// The find of one [`DetailView`](crate::detail::DetailView).
#[derive(Default)]
pub(in crate::detail) struct FindPane {
    /// Whether the strip under the tabs is shown.
    pub(in crate::detail) open: bool,
    /// The text field, made the first time the find opens (it needs a window).
    pub(in crate::detail) input: Option<Entity<InputState>>,
    pub(in crate::detail) subscription: Option<Subscription>,
    /// The pattern as typed.
    pub(in crate::detail) text: String,
    /// The compiled pattern; `None` for no text or text that does not compile.
    pub(in crate::detail) query: Option<FindQuery>,
    /// Why the text is not a pattern. The matches of the last good one stay.
    pub(in crate::detail) error: Option<FilterError>,
    /// The matches' starting bytes and the one the user is on.
    pub(in crate::detail) nav: FindNavigator,
    /// The byte ranges of the matches, in the text they were found in.
    pub(in crate::detail) ranges: Arc<[Range<usize>]>,
    /// Counts scans: only the newest one is applied.
    pub(in crate::detail) generation: u64,
    /// The scan in flight; a newer one replaces (and so cancels) it.
    pub(in crate::detail) task: Option<Task<()>>,
    /// Whether the field has the keyboard focus (the key context says `Editing` meanwhile).
    pub(in crate::detail) editing: bool,
    /// `/` focuses the field at once and sends `resource::Find`; the command coming back is an
    /// echo of that key and does not focus it again.
    pub(in crate::detail) echoes: usize,
}

impl FindPane {
    /// Forgets the matches, keeping the pattern.
    pub(in crate::detail) fn clear_matches(&mut self) {
        self.nav.clear();
        self.ranges = Arc::from([]);
    }
}
