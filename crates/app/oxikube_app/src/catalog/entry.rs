//! [`CatalogEntry`]: one row of the catalog.

use std::cmp::Ordering;

use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{ClusterContext, ClusterSource};

/// One cluster of the catalog: a kubeconfig context, where it came from, and the user's marks.
///
/// Built by [`ClusterCatalog::load`](super::ClusterCatalog::load). A snapshot: it does not
/// follow later changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// The context as the cluster source lists it.
    pub context: ClusterContext,
    /// The source the context came from, when the source list still has it.
    pub source: Option<ClusterSource>,
    /// Whether the user marked it as a favourite.
    pub favourite: bool,
    /// When the user last asked to connect it.
    pub last_used: Option<Timestamp>,
}

impl CatalogEntry {
    /// The catalog id of the entry.
    pub fn id(&self) -> &ClusterId {
        &self.context.cluster
    }

    /// The kubeconfig context name, which is what the user calls the cluster.
    pub fn name(&self) -> &str {
        self.context.context.as_str()
    }

    /// A label for the source file: the source's own label, or its id when the source list no
    /// longer has it.
    pub fn source_label(&self) -> &str {
        match &self.source {
            Some(source) => &source.label,
            None => &self.context.source.0,
        }
    }

    /// The list order when nothing is searched: favourites first, then the most recently used
    /// (never-used entries after used ones), then the name (case-insensitive), then the id so
    /// the order is total and stable.
    pub fn cmp_default(&self, other: &Self) -> Ordering {
        other
            .favourite
            .cmp(&self.favourite)
            .then_with(|| other.last_used.cmp(&self.last_used))
            .then_with(|| self.name().to_lowercase().cmp(&other.name().to_lowercase()))
            .then_with(|| self.name().cmp(other.name()))
            .then_with(|| self.id().cmp(other.id()))
    }
}
