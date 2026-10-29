//! `oxikube_resources_ui` — layer: `ui`.
//!
//! Generic resource table + detail drawer, per-kind panels and actions, create/bulk ops, CRD
//! browsing, apply UI, file browser, audit viewer.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`actions`] | E07-S08 | row actions in the table: the context menu and the palette's list from the `CommandBus` registry ([`actions::ResourceActions`]), disabled in read-only mode, the delete key and [`actions::DeleteDialog`] (propagation choice, type-the-name, bulk delete with per-object results) |
//! | [`audit`] | E19-S09 | [`audit::AuditView`]: the audit log in a virtualised table (cluster / verb / initiator / outcome / time filters pushed into the store query, keyset paging, a live "N new" pill from the `AuditLog` signal, a detail panel with the batch and "Show resource"), `audit::Open` and `audit::Export` (JSONL or CSV) |
//! | [`crds`] | E07-S07 | CRD browsing: [`crds::CrdInfo`] (a CRD read for browsing, the version a table opens), [`crds::served_versions`] (the version switcher's list), [`crds::SchemaTree`] (the `openAPIV3Schema` as a lazy collapsible tree) and the CRD list's row actions |
//! | [`describe_settings`] | E07-S06 | the `describe` setting: which backend renders the Describe tab (`auto`, `native`, `kubectl`) and where `kubectl` is |
//! | [`detail`] | E07-S05 | [`DetailView`](detail::DetailView): the generic detail of one object (header, metadata, owners, conditions, status, YAML and Describe tabs (E07-S06), events), as the right-hand [`DetailDrawer`](detail::DetailDrawer) of a cluster tab or, pinned, a workspace tab |
//! | [`exec`] | E09-S08 | "Shell" and "Attach" on a pod's row and detail: [`exec::exec_row_actions`], the [`exec::ContainerPicker`] for a pod with several containers (the default container, or the last choice for the pod, preselected) and [`exec::ExecFlow`], which reads the pod off the UI thread and dispatches `pod::Shell` / `pod::Attach` with the container chosen |
//! | [`filter`] | E07-S04 | [`FilterBar`](filter::FilterBar): the `/` filter of a table (`/text`, `/!text`, `/-l k=v`, `/-f fuzzy`), its parse error and `123 of 4,812` count, debounce, and the saved filter |
//! | [`kinds`] | E12-S01 | the kind registry's UI half: [`kinds::KindSpec`] (the app's `KindMeta` plus a [`kinds::DetailRenderer`]), the [`kinds::KindSpecs`] global (`kind_for` never fails: unknown kinds get the generic spec), the built-in kinds by sidebar section (each section module's `init`), published to the cluster sidebar, where each cluster gates them by discovery and the visibility hook; the CRD kind's spec and definition panel ([`kinds::crd`], E12-S17); [`kinds::service`] and [`kinds::endpoints`] (E12-S13): Services, Endpoints and EndpointSlices with their detail panel, row actions and keys |
//! | [`kinds::storage`] | E12-S15 | the Storage section: claims (Pods used by), volumes, classes (Default badge) and, where served, volume snapshots; their detail panels; "Resize…" (`pvc::Resize`) and "Set as Default" (`storageclass::SetDefault`) with their dialogs |
//! | [`metrics`] | E13-S03 | live metrics in the views: [`metrics::set_service`] (the app's `MetricsService`), the per-cluster usage sources of the Pod and Node kinds, [`metrics::TableMetrics`] (a Pods or Nodes table watches while active) and the `metrics unavailable: <reason>` badge |
//! | [`navigate`] | E07-S11 | opening a kind's list: the `resource::OpenList` handler and the registry of kind views ([`navigate::KindViews`]) |
//! | [`overview_lite`] | E07-S11 | the Workloads overview: a card per workload kind with its total, a stacked status bar whose segments open the filtered table (E12-S07), and the recent-warnings panel |
//! | [`table`] | E07-S03 | [`ResourceTable`](table::ResourceTable): the generic, virtualised table of one kind, with sorting, column layout per kind, multi-select, context menu and keyboard navigation |
//! | [`table::states`] | E07-S10 | loading / empty / filtered-empty / forbidden / unauthorized / error states, the stale badge, Retry (`resource::RetryFeed`) and the API server's `Warning:` headers as toasts |
//! | [`used_by`] | E12-S18 | "Used by": the shared reference feed ([`used_by::reference_feed`]) and the `resource::ShowUsedBy` row action for ServiceAccounts, ConfigMaps, Secrets, claims and PriorityClasses; the tab is the detail's ([`detail::DetailTab::UsedBy`]) |
//! | [`warnings`] | E14-S04 | the warning badge column of every table ([`warnings::badge_column`]: an icon when the object had a `Warning` event within `events.warning_window`, its tooltip, a click to the Events tab) over the app's `WarningBadges`, installed as [`warnings::ObjectEvents`] |
//! | [`views`] | E07-S03 | [`ResourceViews`]: the kind view that opens tables in cluster tabs (sidebar navigation, `resource::OpenList`) and runs the table commands on the UI thread |
//!
//! The `suite` module (tests only, E07-S12) is the browser's end-to-end suite: the real views driven
//! by scripted feeds over testkit fakes. Its pods-table screenshots live in `tests/screenshot.rs`.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod actions;
pub mod audit;
pub mod bulk;
pub mod crds;
pub mod describe_settings;
pub mod detail;
pub mod exec;
pub mod files;
pub mod filter;
pub mod kinds;
pub mod metrics;
pub mod navigate;
pub mod overview_lite;
pub mod table;
pub mod used_by;
pub mod views;
pub mod warnings;

#[cfg(test)]
mod suite;

pub use describe_settings::{DescribeBackendSetting, DescribeSettings};
pub use views::{
    RESOURCE_COMMANDS, ResourceCommandSink, ResourceViews, ResourceViewsDeps, ResourceViewsSlot,
    ViewRequest, register_commands, sidebar_navigation, view_row_actions,
};

/// Registers what this crate puts in the app: the built-in kinds (their specs and sidebar
/// entries) and the Workloads overview's tiles. Called once from the binary's init, after the
/// workspace's.
pub fn init(cx: &mut gpui::App) {
    kinds::init(cx);
    overview_lite::init(cx);
}
