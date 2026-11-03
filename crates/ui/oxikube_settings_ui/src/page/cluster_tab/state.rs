//! What the page keeps for its per-cluster scope.

use gpui::Task;
use oxikube_app::{CatalogEntry, ClusterCatalog};
use oxikube_domain::ids::ClusterId;

use super::choice::ClusterChoice;
use crate::page::{SearchIndex, SettingsPageModel};

/// Which settings the page edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// The user's `settings.json` as a whole: the page of E21-S04/S05.
    #[default]
    Global,
    /// One cluster's `clusters.<id>` block: the per-cluster tab.
    Cluster,
}

/// The model, search index and selected section of the scope the page is not showing.
pub(crate) struct Parked {
    pub model: SettingsPageModel,
    pub index: SearchIndex,
    pub selected: usize,
}

/// The per-cluster tab's state, owned by the page. The tasks are replaced or dropped only from
/// outside (a new load replaces the old one; the page's drop ends both).
#[derive(Default)]
pub(crate) struct ClusterTabState {
    pub scope: Scope,
    /// The other scope's model and index, swapped in when the scope changes.
    pub parked: Option<Parked>,
    /// The catalog's contexts, in catalog order.
    pub entries: Vec<CatalogEntry>,
    /// The picker's rows: the entries, then blocks the catalog does not list.
    pub choices: Vec<ClusterChoice>,
    /// The cluster picked last, kept across scope switches.
    pub last: Option<ClusterId>,
    pub catalog: Option<ClusterCatalog>,
    pub load: Option<Task<()>>,
    pub watch: Option<Task<()>>,
}
