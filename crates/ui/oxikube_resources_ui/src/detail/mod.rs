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
//! | `find` | find in the YAML and Describe text (E11-S06): `/`, the strip, `n` / `N` through the shared `oxikube_app::search::find` |
//! | `yaml` | the YAML tab (E07-S06): [`yaml_text`] (managedFields, Secret masking) as a pure function run off the UI thread, the read-only code view, the toolbar |
//! | `describe` | the Describe tab (E07-S06): `DescribePort` read on the Tokio bridge, spinner, error with Retry, refresh |
//! | `events` | [`EventRow`], [`events_about`]: the events of the object from the namespace's `Event` feed |
//! | `tabs` | [`DetailTab`]: Overview, YAML, Describe, Events, and a CRD's Schema |
//! | `state` | [`DetailDeps`], [`Mount`], [`DetailState`]: what the view is built over and how the object stands |
//! | `keys` | the `Detail` key context and its actions (E07-U559): close, step the table, switch tab |
//! | `ages` | the age tick (E07-F566): a shown detail redraws for ages only when one reads differently |
//! | `view` | [`DetailView`]: the entity and its commands |
//! | `follow` | the store subscription (one row on the table's own feed), the model and the list's rows kept in step |
//! | `full` | the full read for metadata-only and Table feeds, and the owners' scopes from discovery |
//! | `exec` | "Shell" and "Attach" buttons in a pod's header (E09-S08), through the same [`ExecFlow`](crate::exec::ExecFlow) as the table's row actions |
//! | `scale` | `scale_state` and `open_scale` (E12-S04): whether the view offers Scale and the flow that opens its dialog, for the workload panel's button and the palette |
//! | `rollout` | "Restart" and "Set image" in the header of a Deployment, StatefulSet or DaemonSet (E12-S05): `rollout_state`, the buttons, `workload::Restart` through the bus and the set-image flow's dialog, also for the palette |
//! | `rollback` | `rollback_state` and `open_rollback` (E12-S06): whether the view offers Rollback and the flow that opens its dialog, for the workload panel's button and the palette |
//! | `editor_buttons` | "Edit" and "View YAML in Editor" in the header (E10-S06): `resource::Edit` / `resource::ViewYamlInEditor`, which open a `kind/name` manifest editor tab |
//! | `events_feed` | the Events tab's subscription, started on first show |
//! | `render`, `overview`, `parts`, `events_tab` | drawing: header, tab strip, the virtualised Overview (its stateless rows in `parts`) and Events lists |
//! | `schema_tab`, `schema_view` | the Schema tab of a CRD's detail (E07-S07): its `openAPIV3Schema` as a collapsible tree, the version chips and the way to the custom resources (state and commands, then drawing) |
//! | `reveal` | revealing a Secret's values in memory (E12-S11): the summary a Secret's full read keeps, `secret::Reveal` (`x`), the hide-again timer, the decoded values the YAML tab shows |
//! | `kind_panel` | the kind's own panel above the Overview, from its spec's [`DetailRenderer`](crate::kinds::DetailRenderer) (E12-S01), and [`DetailView::resource`], what the panel reads |
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

mod ages;
mod describe;
mod drawer;
mod editor_buttons;
mod events;
mod events_feed;
mod events_tab;
mod exec;
mod find;
mod follow;
mod full;
mod keys;
mod kind_panel;
pub mod model;
mod overview;
mod parts;
mod render;
mod reveal;
mod rollback;
mod rollout;
mod scale;
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
pub use keys::{
    Close, CloseFind, EditSecretDecoded, Find, NextMatch, PreviousMatch, RevealSecret, SelectNext,
    SelectPrevious, ShowTab,
};
pub use kind_panel::PanelHost;
pub use model::DetailModel;
pub use reveal::{REVEAL_TIMEOUT, RevealLoad};
pub use state::{DetailDeps, DetailEvent, DetailState, Mount};
pub use tabs::DetailTab;
pub use view::{DetailView, item_key};
pub use yaml::{YamlOptions, has_managed_fields, yaml_text, yaml_text_revealed};
