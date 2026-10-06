//! CRD browsing (E07-S07): any custom resource is browsable on day one, with kubectl's columns.
//!
//! Custom resources use the generic [`ResourceTable`](crate::table::ResourceTable) and detail
//! drawer unchanged; what is specific to CRDs lives here, as plain Rust with no GPUI:
//!
//! | File | Holds |
//! |---|---|
//! | `info` | [`CrdInfo`]: a CRD object as the browser needs it (group, kind, scope, names, versions) and the version a table opens ([`CrdInfo::display_version`]) |
//! | `versions` | [`served_versions`]: the versions of a kind that discovery serves, newest first, for the table's version switcher |
//! | `schema` | [`SchemaTree`]: `openAPIV3Schema` of one version as a lazily built, collapsible tree (type, description, required, enum), truncated when it is deep or huge |
//! | `actions` | [`crd_row_actions`]: the row actions of a CRD table (open its custom resources, show its details) |
//!
//! # How a user gets here
//!
//! The sidebar's Custom Resources section lists the cluster's API groups with how many kinds
//! each has, collapsed; a kind's entry opens its table (`resource::OpenList`), the section's
//! "Definitions" entry opens the CRD list (`crd::OpenList`), and a row of that list opens the
//! table of the custom resources it defines (`crd::OpenResources`: Enter, double click or the
//! row menu). The CRD's own detail drawer has a Schema tab, and a button that does the same.
//!
//! # Feeds
//!
//! A custom resource table is on the server-side Table feed (ADR 0006: the columns are
//! `kubectl get`'s, `additionalPrinterColumns` included). Nothing here starts a feed: counts in
//! the sidebar read feeds that are open, and the CRD list is an ordinary reflector feed.

mod actions;
mod info;
pub mod schema;
mod versions;

#[cfg(test)]
pub(crate) mod tests;

pub use actions::crd_row_actions;
pub use info::{CrdInfo, CrdVersion, crd_gvk, is_crd_kind, version_order};
pub use schema::{
    MAX_DEPTH, MAX_ROWS, RowKind, SchemaRow, SchemaRows, SchemaTree, schema_root, version_names,
};
pub use versions::served_versions;
