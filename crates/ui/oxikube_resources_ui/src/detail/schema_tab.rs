//! The Schema tab of a CRD's detail (E07-S07): the `openAPIV3Schema` of one served version as a
//! collapsible tree, and the way into the custom resources the CRD defines.
//!
//! The tree is [`SchemaTree`]: lazy (only open nodes are walked) and bounded. The view keeps its
//! visible rows and a list state; a click on a row opens or closes it, which re-walks the open
//! nodes once and tells the list which rows changed, so the scroll stays where it was. Nothing is
//! computed in render. The rows are rebuilt when the CRD changes, never per frame.
//!
//! The walk wants the CRD as a [`Value`] tree, which is several times the size of the stored
//! document, so the tree is decoded once per CRD version and kept for as long as the Schema tab is
//! up: a click on a row or a version chip re-walks the cached tree and decodes nothing. While
//! another tab is shown nothing is decoded or kept, and the tab is rebuilt when it is opened.

use gpui::{Context, ListAlignment, ListState, px};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use serde_json::Value;

use super::tabs::DetailTab;
use super::view::DetailView;
use crate::crds::{CrdInfo, SchemaRow, SchemaRows, SchemaTree, schema_root, version_names};

/// What the Schema tab keeps: the CRD as read for browsing, the version shown, which nodes are
/// open and the rows that follow.
pub(super) struct SchemaPane {
    pub(super) info: Option<CrdInfo>,
    /// The versions that declare a schema, in the CRD's order.
    pub(super) versions: Vec<String>,
    /// The version shown; `None` until the CRD is known.
    pub(super) version: Option<String>,
    pub(super) tree: SchemaTree,
    pub(super) rows: SchemaRows,
    pub(super) list: ListState,
    /// The CRD decoded for the walk: set while the Schema tab is up, dropped when it is left.
    crd: Option<Value>,
    /// The CRD changed (or arrived) while another tab was shown: the tab reads it when opened.
    stale: bool,
    /// How many times the CRD was decoded into a tree.
    #[cfg(test)]
    pub(super) decodes: usize,
}

impl Default for SchemaPane {
    fn default() -> Self {
        Self {
            info: None,
            versions: Vec::new(),
            version: None,
            tree: SchemaTree::new(),
            rows: SchemaRows::default(),
            list: ListState::new(0, ListAlignment::Top, px(120.)),
            crd: None,
            stale: false,
            #[cfg(test)]
            decodes: 0,
        }
    }
}

impl SchemaPane {
    /// The visible rows of `version`'s schema in `crd`, walking only the open nodes (none when the
    /// version is unknown or has no schema).
    fn walk(&self, crd: &Value, version: Option<&str>) -> SchemaRows {
        version
            .and_then(|v| schema_root(crd, v))
            .map(|root| self.tree.rows(root))
            .unwrap_or_default()
    }

    /// Whether the decoded CRD is held (only while the Schema tab is up).
    #[cfg(test)]
    pub(super) fn is_decoded(&self) -> bool {
        self.crd.is_some()
    }

    /// Takes `rows` as the visible ones, and tells the list what changed.
    fn replace_rows(&mut self, rows: SchemaRows) {
        let old = std::mem::replace(&mut self.rows, rows);
        sync_list(&self.list, &old.rows, &self.rows.rows);
    }
}

/// Tells `list` which rows of `old` became `new`: the middle that differs is spliced, the rest is
/// remeasured in place, so the scroll position stays.
fn sync_list(list: &ListState, old: &[SchemaRow], new: &[SchemaRow]) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if old.len() != new.len() || prefix + suffix < old.len() {
        list.splice(prefix..old.len() - suffix, new.len() - prefix - suffix);
    }
    if !new.is_empty() {
        list.remeasure_items(0..new.len());
    }
}

impl DetailView {
    /// Decodes the CRD into the pane's cache, once the object is complete (a Table-feed CRD waits
    /// for its full read). `false` when there is nothing to decode yet.
    fn decode_crd(&mut self) -> bool {
        let from_feed = self
            .object
            .as_deref()
            .and_then(|object| object.resource())
            .filter(|resource| !resource.is_partial());
        let Some(resource) = from_feed.or(self.full.resource()) else {
            return false;
        };
        self.schema.crd = Some(Resource::to_value(resource));
        #[cfg(test)]
        {
            self.schema.decodes += 1;
        }
        true
    }

    /// Reads the CRD again after the object changed: its versions, the one shown (kept when it is
    /// still there, else the CRD's display version) and the rows. Does nothing for another kind.
    /// While another tab is shown this only notes that the Schema tab is out of date: decoding a
    /// multi-megabyte CRD for a tab nobody looks at would be a stall on the UI thread for nothing.
    pub(super) fn rebuild_schema(&mut self) {
        if !crate::crds::is_crd_kind(&self.target.gvk) {
            return;
        }
        if self.tab != DetailTab::Schema {
            self.schema.stale = true;
            self.schema.crd = None;
            return;
        }
        if !self.decode_crd() {
            return;
        }
        self.schema.stale = false;
        let Some(json) = self.schema.crd.as_ref() else {
            return;
        };
        let info = CrdInfo::parse(json);
        let versions = version_names(json);
        let keep = self.schema.version.clone().filter(|v| versions.contains(v));
        let version = keep.or_else(|| {
            let shown = info.as_ref().and_then(|i| i.display_version());
            shown
                .map(|v| v.name.clone())
                .filter(|name| versions.contains(name))
                .or_else(|| versions.first().cloned())
        });
        let rows = self.schema.walk(json, version.as_deref());
        self.schema.info = info;
        self.schema.versions = versions;
        self.schema.version = version;
        self.schema.replace_rows(rows);
    }

    /// Called when the Schema tab is shown: reads the CRD if it changed while the tab was away.
    pub(super) fn schema_tab_opened(&mut self) {
        if self.schema.stale || self.schema.info.is_none() {
            self.rebuild_schema();
        }
    }

    /// Called when the Schema tab is left: the decoded tree is not kept for a tab nobody shows.
    pub(super) fn schema_tab_closed(&mut self) {
        self.schema.crd = None;
    }

    /// The CRD as read for the Schema tab (`None` for another kind, and until the object is known).
    pub fn crd_info(&self) -> Option<&CrdInfo> {
        self.schema.info.as_ref()
    }

    /// The versions that declare a schema, in the CRD's order.
    pub fn schema_versions(&self) -> &[String] {
        &self.schema.versions
    }

    /// The version whose schema the tab shows.
    pub fn schema_version(&self) -> Option<&str> {
        self.schema.version.as_deref()
    }

    /// The visible rows of the schema tree.
    pub fn schema_rows(&self) -> &[SchemaRow] {
        &self.schema.rows.rows
    }

    /// Whether the tree stopped at its row limit.
    pub fn schema_truncated(&self) -> bool {
        self.schema.rows.truncated
    }

    /// Shows the schema of `version` (one the CRD declares a schema for); the open nodes stay
    /// open where the other version has them too.
    pub fn set_schema_version(&mut self, version: &str, cx: &mut Context<Self>) {
        if self.schema.version.as_deref() == Some(version)
            || !self.schema.versions.iter().any(|v| v == version)
        {
            return;
        }
        self.schema.version = Some(version.to_owned());
        self.reread_schema_rows();
        cx.notify();
    }

    /// Opens or closes the schema node `key`. `false` when no visible row has that key.
    pub fn toggle_schema(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let known = self
            .schema
            .rows
            .rows
            .iter()
            .any(|row| &*row.key == key && row.expandable);
        if !known {
            return false;
        }
        self.schema.tree.toggle(key);
        self.reread_schema_rows();
        cx.notify();
        true
    }

    /// Walks the open nodes of the shown version again.
    fn reread_schema_rows(&mut self) {
        if self.schema.crd.is_none() && !self.decode_crd() {
            return;
        }
        let Some(json) = self.schema.crd.as_ref() else {
            return;
        };
        let rows = self.schema.walk(json, self.schema.version.as_deref());
        self.schema.replace_rows(rows);
    }

    /// Opens the table of the custom resources this CRD defines: sends `crd::OpenResources`.
    pub fn open_custom_resources(&mut self, cx: &mut Context<Self>) {
        let command = Command::CrdOpenResources {
            cluster: self.target.cluster.clone(),
            name: self.target.name.to_string(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }
}
