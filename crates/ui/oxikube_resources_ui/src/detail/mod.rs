//! The generic detail drawer (E07-S05): everything about one object, for any kind including
//! custom resources, without leaving the table.
//!
//! [`DetailView`] is one entity with two mounting modes ([`Mount`]): the content of the
//! [`DetailDrawer`] (the right dock of a cluster tab) and, after "Pin as tab", a workspace
//! [`Item`](oxikube_workspace::Item) that moves between panes. The active tab, the scroll and the
//! expanded values live in the entity, so promoting it resets nothing.
//!
//! | File | Holds |
//! |---|---|
//! | `model` | [`DetailModel`]: header, labels, annotations, owners, finalizers, conditions, the `status` summary and a Secret's key names; plain Rust, no GPUI |
//! | `yaml` | the YAML tab (E07-S06): [`yaml_text`] (managedFields, Secret masking) as a pure function, the read-only highlighted editor, the toolbar |
//! | `describe` | the Describe tab (E07-S06): `DescribePort` read on the Tokio bridge, spinner, error with Retry, refresh |
//! | `events` | [`EventRow`], [`events_about`]: the events of the object from the namespace's `Event` feed |
//! | `tabs` | [`DetailTab`]: Overview, YAML, Describe, Events, and a CRD's Schema |
//! | `state` | [`DetailDeps`], [`Mount`], [`DetailState`]: what the view is built over and how the object stands |
//! | `view` | [`DetailView`]: the entity and its commands |
//! | `follow` | the store subscription (one row on the table's own feed), the model and the list's rows kept in step |
//! | `full` | the full read for metadata-only and Table feeds, and the owners' scopes from discovery |
//! | `events_feed` | the Events tab's subscription, started on first show |
//! | `render`, `overview`, `parts`, `events_tab` | drawing: header, tab strip, the virtualised Overview (its stateless rows in `parts`) and Events lists |
//! | `schema_tab`, `schema_view` | the Schema tab of a CRD's detail (E07-S07): its `openAPIV3Schema` as a collapsible tree, the version chips and the way to the custom resources (state and commands, then drawing) |
//! | `drawer` | [`DetailDrawer`]: the `Panel` that hosts a detail and hands it to the workspace when pinned |
//!
//! # How a user gets here
//!
//! Double-click or Enter on a table row (`resource::Open`), or an owner link in another drawer.
//! [`ResourceViews`](crate::ResourceViews) opens the drawer in the cluster's tab; "Pin as tab"
//! (`resource::PinDetail`) promotes it.
//!
//! # What runs where
//!
//! Nothing blocks the UI thread. The object arrives as a one-row subscription on the feed the
//! table already holds (no extra watch); kinds whose feed has no `spec` or `status` are read once
//! on the Tokio bridge with `spawn_kube`, with Secret values removed inside that task. Owner scopes
//! come from discovery, also off-thread; the Events feed starts when its tab is first shown.
//! Redraws from feeds are coalesced to frame cadence, and the long bodies are virtualised.
//!
//! # Secrets
//!
//! A Secret shows its key names and never a value ([`model::mask_secret`]); its
//! `last-applied-configuration` annotation, which embeds the data, is dropped.

mod describe;
mod drawer;
mod events;
mod events_feed;
mod events_tab;
mod follow;
mod full;
pub mod model;
mod overview;
mod parts;
mod render;
mod schema_tab;
mod schema_view;
mod state;
mod tabs;
mod view;
mod yaml;

#[cfg(test)]
pub(crate) mod tests;

pub use describe::DescribeState;
pub use drawer::{DEFAULT_WIDTH, DetailDrawer, ToggleDrawer};
pub use events::{EventRow, MAX_EVENTS, events_about};
pub use model::DetailModel;
pub use state::{DetailDeps, DetailEvent, DetailState, Mount};
pub use tabs::DetailTab;
pub use view::{DetailView, item_key};
pub use yaml::{YamlOptions, has_managed_fields, yaml_text};
