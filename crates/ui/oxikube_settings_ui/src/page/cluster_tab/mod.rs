//! The per-cluster tab of the settings page (E21-S06): pick a cluster, see and edit the settings
//! a `clusters.<id>` block may override, and see which layer wins.
//!
//! It is the same page, switched to its "Per cluster" scope (the two buttons under the title):
//! the sections, the search, the controls ([`crate::controls`]) and the layer badge are the
//! page's own, pointed at a different model and at a [`ClusterId`](oxikube_domain::ids::ClusterId).
//!
//! * **Which fields.** The schema marks the per-cluster settings (`x-oxikube-cluster-scoped`,
//!   [`CLUSTER_SCOPED_KEY`](oxikube_settings::CLUSTER_SCOPED_KEY)); [`cluster_page_model`] keeps
//!   exactly those, with the object-valued ones (`prometheus`, `guard`, ...) as sections of their
//!   own. No field name is spelled out here. Any other setting is accepted under `clusters.<id>`
//!   by the store, but is not offered per cluster.
//! * **Which clusters.** The catalog's contexts ([`ClusterChoice`]), plus any `clusters.<id>`
//!   block in the file whose cluster the catalog no longer lists (so a stale block can be cleared).
//!   Ids are opaque hashes, so a cluster is shown by its `display_name` override, else its context
//!   name.
//! * **Writes.** The page's controls write `clusters.<id>.<path>` through
//!   [`update_user_settings_path`](oxikube_settings::update_user_settings_path); "Reset for this
//!   cluster" removes the key, and the last key of a block removes the block.
//! * **Precedence.** A panel explains `default.json`, then `settings.json`, then `clusters.<id>`;
//!   each field shows what every layer says and which one wins ([`Precedence`]), and a warning
//!   (never a block) when an override weakens a protection ([`protection_warning`]).
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`cluster_page_model`]: the per-cluster sections, from the schema |
//! | `choice` | [`ClusterChoice`]: the picker's rows, and [`ClusterRowText`], what each shows (worked out per change, not per frame) |
//! | `precedence` | [`Precedence`], [`protection_warning`] and the explanation text: plain data |
//! | `state` | `ClusterTabState`: scope, the other scope's model, the catalog and its tasks |
//! | `view` | scope switching, the catalog and the picked cluster |
//! | `render` | the scope switch, the picker (a `uniform_list`: only the rows in view are built) and the precedence strip on the page |

mod choice;
mod model;
mod precedence;
mod render;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use choice::{ClusterChoice, ClusterRowText, choices};
pub use model::{cluster_page_model, supports_cluster_scope};
pub use precedence::{EXPLANATION, Precedence, PrecedenceCell, protection_warning};
pub(crate) use state::ClusterTabState;
pub use state::Scope;
