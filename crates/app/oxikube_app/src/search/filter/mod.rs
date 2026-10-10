//! The `/` filter of a table as the rest of the app sees it (E11-S06): one grammar, one parser,
//! one small state value, shared by the table's filter bar and the `:` jump bar.
//!
//! The grammar and the matching were written for the filter bar (E07-S04) and live with the store
//! they run in ([`crate::store::filter`]); this module is where the rest of E11 reaches them, so
//! `:pod /re` and a `/re` typed in the table agree because they call the same [`parse`].
//!
//! | Input | Meaning | Runs |
//! |---|---|---|
//! | `re`, `/re` | the names matching the regex (a plain substring when it has no regex syntax), case-insensitive | client, over the cached rows |
//! | `!re`, `/!re` | the names that do not match | client |
//! | `-l app=x`, `/-l tier in (a,b)` | a label selector (`=`, `==`, `!=`, `in`, `notin`, `key`, `!key`) | **server**: the feed is re-keyed with the selector |
//! | `-f wb`, `/-f wb` | fuzzy: a subsequence of the name, best match first | client, ranked |
//! | `re -l app=x` | a name filter and a selector together | each where it runs |
//! | `/`, empty | no filter | |
//!
//! # Choices
//!
//! * **What a pattern is matched against**: the object's name (what a row is identified by, and
//!   what the store caches without building any cell text). k9s matches the whole rendered row;
//!   here the other columns are filtered with a selector (`-l`) or by sorting, and a free-text
//!   match over cells would mean formatting every cell of every row per keystroke. Case is
//!   ignored.
//! * **A pattern that starts with `-` or `!`** is searched by escaping it: `\-x`, `\!x`.
//! * **A pattern with spaces** is kept whole (names have none, so it matches nothing); only a
//!   `-l` word after a name starts the selector.
//! * **Errors keep the last good filter**: [`FilterState::edit`] stores the error beside the last
//!   good parse, so the table keeps its rows and shows the message.
//! * **Limits**: the input is at most [`MAX_FILTER_LEN`] characters, and a compiled regex at most
//!   1 MiB, so a pasted blob or a nested repeat cannot stall typing.

mod state;

#[cfg(test)]
mod tests;

pub use state::FilterState;

pub use crate::store::filter::{
    FilterError, FilterExpr, FilterParts, Fuzzy, MAX_FILTER_LEN, NameFilter, NameMatcher,
    TextPattern, parse,
};
