//! The settings page (E21-S04): every registered setting, by section, with a search and the
//! layer each value comes from.
//!
//! Open it with `settings::Open` (`cmd-,` / `ctrl-,`, the application menu's Settings…, the
//! palette's "Open Settings", an agent's `app.settings_open`): a tab of the main window, not of a
//! cluster, opened once (asking again shows the open tab).
//!
//! * The left navigation lists the sections: "General" (the top-level fields that are not
//!   objects: theme, keymap, UI scale, the defaults of every cluster) and one per top-level object
//!   of `settings.json` (`terminal`, `editor`, `helm`, ...). They come from the settings schema,
//!   so a crate that registers a setting adds its section with no code here.
//! * The search field matches section titles, field titles, paths and descriptions fuzzily
//!   (the palette's engine); a result opens its section scrolled to the field, highlighted.
//! * Each field shows a badge for the layer its value comes from (`Default`, `User`, `Cluster`)
//!   with a tooltip on how the layers stack, and the control that edits it ([`crate::controls`],
//!   E21-S05); the per-cluster tab ([`cluster_tab`], E21-S06) edits one cluster's block instead.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`SettingsPageModel`]: sections and fields as plain data, built from the schema |
//! | `search` | [`SearchIndex`]: the fuzzy search over the model, built once per page open |
//! | `badge` | [`LayerBadge`]: the badge's words for a [`SettingSource`](oxikube_settings::SettingSource) |
//! | `cluster_tab` | the per-cluster scope (E21-S06): the cluster list, the precedence strip |
//! | `view` | [`SettingsPage`]: the entity, its frame and the workspace item |
//!
//! Built from `oxikube_ui` primitives rather than gpui-component's `setting::Settings`: that
//! widget filters by substring, renders every page it is given and has no place for a per-field
//! badge, while this page needs the shared fuzzy engine, a jump to a field and lazy sections.

mod badge;
pub mod cluster_tab;
mod model;
pub(crate) mod schema_walk;
mod search;
mod titles;
mod view;

#[cfg(test)]
pub(crate) mod harness;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

pub use badge::LayerBadge;
pub use cluster_tab::{
    ClusterChoice, ClusterRowText, Scope, cluster_page_model, supports_cluster_scope,
};
pub use model::{FieldKind, PageField, PageSection, SettingsPageModel};
pub use search::{MAX_HITS, MatchedIn, SearchHit, SearchIndex};
pub use titles::{GENERAL_KEY, humanize};
pub use view::{SETTINGS_VIEW, SettingsPage};
