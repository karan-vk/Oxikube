//! [`Row`]: a catalog entry with its display strings and search text prepared once.

use gpui::SharedString;
use oxikube_app::CatalogEntry;

/// What the view shows for a missing cluster or user name.
const NONE: &str = "-";

/// A cluster or user name for display, `-` when the context names none.
fn or_none(name: Option<&str>) -> SharedString {
    SharedString::from(name.unwrap_or(NONE).to_owned())
}

/// One catalog entry, prepared when the entries are set so rendering and filtering do no string
/// building per frame or per keystroke.
#[derive(Debug, Clone)]
pub struct Row {
    entry: CatalogEntry,
    name: SharedString,
    cluster: SharedString,
    user: SharedString,
    source: SharedString,
    /// The text the fuzzy matcher searches: every column joined by spaces.
    haystack: String,
}

impl Row {
    pub(super) fn new(entry: CatalogEntry) -> Self {
        let name = SharedString::from(entry.name().to_owned());
        let cluster = or_none(entry.context.cluster_name.as_deref());
        let user = or_none(entry.context.user.as_deref());
        let source = SharedString::from(entry.source_label().to_owned());
        let mut haystack =
            String::with_capacity(name.len() + cluster.len() + user.len() + source.len() + 3);
        for column in [&name, &cluster, &user, &source] {
            if !haystack.is_empty() {
                haystack.push(' ');
            }
            haystack.push_str(column);
        }
        Self {
            entry,
            name,
            cluster,
            user,
            source,
            haystack,
        }
    }

    /// The catalog entry.
    pub fn entry(&self) -> &CatalogEntry {
        &self.entry
    }

    /// The context name, which is what the user calls the cluster.
    pub fn name(&self) -> &SharedString {
        &self.name
    }

    /// The kubeconfig cluster entry the context points at (`-` when it names none).
    pub fn cluster(&self) -> &SharedString {
        &self.cluster
    }

    /// The kubeconfig user entry the context points at (`-` when it names none).
    pub fn user(&self) -> &SharedString {
        &self.user
    }

    /// The source file label.
    pub fn source(&self) -> &SharedString {
        &self.source
    }

    pub(super) fn haystack(&self) -> &str {
        &self.haystack
    }
}
