//! Search over names the user types (E11): the alias table behind the `:` jump bar.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`aliases`] | E11-S04 | [`aliases::AliasTable`]: built-in k9s aliases, aliases derived from API discovery, the user's `aliases.json`; [`aliases::AliasRegistry`], one table per cluster |
//! | [`jump`] | E11-S05 | [`jump::parse`], [`jump::plan`]: the `:` bar's grammar, parser, the navigation `Command`s a line stands for, history (`-`, `[`, `]`) and completion |
//!
//! Plain Rust, no gpui: the jump bar (`oxikube_palette::jump`), the help overlay (E11-S10) and an agent's tools
//! read the same tables.

pub mod aliases;
pub mod jump;
