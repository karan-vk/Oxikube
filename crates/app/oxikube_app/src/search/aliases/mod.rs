//! The alias table behind the `:` jump bar (E11-S04): `:deploy`, `:pods`, `:certs`, `:fred`.
//!
//! | Piece | Where |
//! |---|---|
//! | [`AliasTable`]: resolve a word to a [`Resolution`], rebuild after discovery, push user aliases | `table` |
//! | [`AliasRegistry`]: one table per cluster, the user's aliases for all of them; [`AliasFollow`] keeps the tables in step with the sessions' API discovery | `registry`, `follow` |
//! | [`AliasEntry`], [`AliasSource`], [`Resolution`], [`AliasConflict`] | `entry` |
//! | the k9s table as data | `builtin` |
//! | the discovery layer, by API group | `discovered` |
//! | the merged lookup and its collision rules | `index`; suggestions for an unknown word in `suggest` |
//!
//! # Layers and precedence
//!
//! `User > BuiltIn > Discovery`. The user's `aliases.json` is the last word; the built-in k9s
//! names (`po`, `dp`, `svc`, ...) come next so a CRD's short name cannot take `dp` from
//! Deployments; the cluster's discovery (plural, singular, short names and Kind of every served
//! type, and `plural.group`) fills in the rest. A built-in follows the version the cluster
//! prefers (`hpa` is `autoscaling/v2` on a current server).
//!
//! # Collisions are never silent
//!
//! - A name in a higher layer hides lower entries that lead elsewhere: resolved there, recorded
//!   as [`ConflictKind::Shadowed`] in [`AliasTable::conflicts`].
//! - Several served types claiming one name (two CRDs with the same plural in different groups):
//!   [`Resolution::Ambiguous`] with the candidates in a fixed order (core group first, then by
//!   group), so the first is the same on every run whatever order discovery answered in, and
//!   recorded as [`ConflictKind::Ambiguous`]. `plural.group` always names one of them.
//!
//! The user file is parsed by `oxikube_settings` (platform), which this crate cannot depend on:
//! the binary pushes the parsed entries in with [`AliasRegistry::set_user_aliases`].

mod builtin;
mod discovered;
mod entry;
mod follow;
mod index;
mod registry;
mod suggest;
mod table;
#[cfg(test)]
mod tests;

pub use entry::{AliasConflict, AliasEntry, AliasSource, ConflictKind, Resolution};
pub use follow::AliasFollow;
pub use registry::AliasRegistry;
pub use suggest::MAX_SUGGESTIONS;
pub(crate) use suggest::closest_names;
pub use table::AliasTable;
