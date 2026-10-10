//! Find in the text a detail shows (E11-S06): `/` opens a field over the YAML or Describe text,
//! every match is coloured, and `n` / `N` step through them, wrapping at the ends.
//!
//! | File | Holds |
//! |---|---|
//! | `state` | [`FindPane`]: the field, the compiled query, the matches and the current one |
//! | `ops` | the view's methods: `find`, typing, `next_match` / `previous_match`, the scan, and the requests that send the `resource::*` commands |
//! | `bar` | the strip under the tabs: the field, `3 / 12`, previous and next, close |
//!
//! The matching is `oxikube_app::search::find`: one rule for `n` / `N` shared with the log view.
//! The scan runs on the background executor over the same `Arc<str>` the code view shows (a
//! 10 MB YAML is a few milliseconds), a newer edit replaces the scan in flight, and the code view
//! draws the ranges only while that text is the one on screen. A key, a button, the palette and an
//! agent all go through the bus: `request_*` sends `resource::Find`, `resource::NextMatch` and
//! `resource::PreviousMatch` with the view's target, and [`ResourceViews`](crate::ResourceViews)
//! calls the operation. Typing in the field is text editing and applies at once. Nothing here
//! reads or changes a cluster, so no `MutationGuard` tier applies.

mod bar;
mod ops;
mod state;

#[cfg(test)]
mod tests;

pub(super) use state::FindPane;
