//! Search over names the user types (E11): the alias table behind the `:` jump bar.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`aliases`] | E11-S04 | [`aliases::AliasTable`]: built-in k9s aliases, aliases derived from API discovery, the user's `aliases.json`; [`aliases::AliasRegistry`], one table per cluster |
//!
//! Plain Rust, no gpui: the jump bar (E11-S05), the help overlay (E11-S10) and an agent's tools
//! read the same tables.

pub mod aliases;
