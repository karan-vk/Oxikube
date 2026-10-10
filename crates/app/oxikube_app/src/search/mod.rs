//! Search over names the user types (E11): the alias table behind the `:` jump bar.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`aliases`] | E11-S04 | [`aliases::AliasTable`]: built-in k9s aliases, aliases derived from API discovery, the user's `aliases.json`; [`aliases::AliasRegistry`], one table per cluster |
//! | [`filter`] | E11-S06 | [`filter::parse`], [`filter::FilterState`]: the `/` filter grammar of tables (`re`, `!re`, `-l selector`, `-f fuzzy`), shared by the filter bar and the jump bar; the grammar itself is `crate::store::filter` |
//! | [`find`] | E11-S06 | [`find::FindNavigator`]: `n` / `N` over the matches of a text, wrapping, shared by the log and YAML views |
//! | [`jump`] | E11-S05 | [`jump::parse`], [`jump::plan`]: the `:` bar's grammar, parser, the navigation `Command`s a line stands for, history (`-`, `[`, `]`) and completion |
//!
//! Plain Rust, no gpui: the jump bar (`oxikube_palette::jump`), the help overlay (E11-S10) and an agent's tools
//! read the same tables.

pub mod aliases;
pub mod filter;
pub mod find;
pub mod jump;
