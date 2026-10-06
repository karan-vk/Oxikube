//! [`NamespaceCatalog`]: the namespaces the selector offers, and how they were found.

use oxikube_domain::ids::Gvk;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{ListOptions, ResourceReader};

/// Pages are small and namespaces few; the cap only stops a server that never ends a list.
const PAGE_SIZE: u32 = 500;
const MAX_PAGES: usize = 100;

/// Where the offered names came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamespaceSource {
    /// The cluster listed its namespaces.
    Cluster,
    /// The cluster refused (`403`, RBAC): the names are the ones the user typed. The selector
    /// says so and offers a field to type more.
    Forbidden,
    /// The list could not be read (not connected, network, unsupported): the names are the
    /// typed ones; the selection and favourites still work.
    Unavailable,
}

/// The namespaces to offer, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceCatalog {
    /// The names, sorted and unique.
    pub names: Vec<String>,
    /// Where they came from.
    pub source: NamespaceSource,
}

impl NamespaceCatalog {
    /// Whether `name` is offered.
    pub fn contains(&self, name: &str) -> bool {
        self.names
            .binary_search_by(|n| n.as_str().cmp(name))
            .is_ok()
    }

    /// Whether the names are the cluster's own list, so a missing name is truly gone.
    pub fn is_authoritative(&self) -> bool {
        self.source == NamespaceSource::Cluster
    }

    /// A catalog of `names` (sorted, deduplicated) from `source`.
    pub(super) fn new(mut names: Vec<String>, source: NamespaceSource) -> Self {
        names.sort();
        names.dedup();
        Self { names, source }
    }

    /// A catalog with no names and no list: what a selector shows until the first answer.
    pub fn unlisted() -> Self {
        Self::new(Vec::new(), NamespaceSource::Unavailable)
    }

    /// Adds `name` (one the user typed) to a catalog that is not the cluster's own list.
    /// Returns whether it was added. A cluster's own list never takes typed names.
    pub fn insert_typed(&mut self, name: &str) -> bool {
        if self.is_authoritative() || self.contains(name) {
            return false;
        }
        self.names.push(name.to_owned());
        self.names.sort();
        true
    }

    /// The source a failed list maps to.
    pub(super) fn source_for(kind: ErrorKind) -> NamespaceSource {
        if kind == ErrorKind::Forbidden {
            NamespaceSource::Forbidden
        } else {
            NamespaceSource::Unavailable
        }
    }
}

/// Lists every namespace name through `reader` (metadata only, paged).
pub(super) async fn list_names(reader: &dyn ResourceReader) -> OxiResult<Vec<String>> {
    let kind = Gvk::new("", "v1", "Namespace");
    let mut names = Vec::new();
    let mut options = ListOptions::default().limit(PAGE_SIZE);
    for _ in 0..MAX_PAGES {
        let page = reader.list_metadata(&kind, None, &options).await?;
        names.extend(page.items.iter().map(|meta| meta.name.to_string()));
        match page.continue_token.as_deref() {
            Some(token) if !token.is_empty() => {
                options = ListOptions::default().limit(PAGE_SIZE).continue_from(token);
            }
            _ => break,
        }
    }
    Ok(names)
}

/// Whether `name` is a valid namespace name (an RFC 1123 label: lowercase alphanumerics and
/// `-`, starting and ending alphanumeric, at most 63 characters). Typed names are checked
/// with it.
pub fn is_valid_namespace_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let edge = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    !bytes.is_empty()
        && bytes.len() <= 63
        && edge(bytes[0])
        && edge(bytes[bytes.len() - 1])
        && bytes.iter().all(|&b| edge(b) || b == b'-')
}
