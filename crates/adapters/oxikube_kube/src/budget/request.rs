//! [`FeedRequest`]: which feed a subscriber wants, and the key feeds are shared by.

use oxikube_domain::ids::Gvk;
use oxikube_ports::{FeedVariant, IncludeObject, ListOptions, TableOptions, WatchOptions};

/// One feed: a kind in one namespace (or the whole cluster), a [`FeedVariant`] and optional
/// selectors.
///
/// Two equal requests share one feed. A namespace selection of several namespaces is several
/// requests, one per namespace ([`SelectionLease`](super::SelectionLease)), so a change of
/// selection keeps the feeds of the namespaces that stay selected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedRequest {
    /// The kind to watch.
    pub gvk: Gvk,
    /// The namespace to watch; `None` for the whole cluster (and for cluster-scoped kinds).
    pub namespace: Option<String>,
    /// What the feed carries. The budget may grant [`FeedVariant::Metadata`] for a
    /// [`FeedVariant::Full`] request.
    pub variant: FeedVariant,
    /// Label selector.
    pub label_selector: Option<String>,
    /// Field selector.
    pub field_selector: Option<String>,
}

impl FeedRequest {
    /// A cluster-wide feed of `gvk` carrying `variant`, with no selectors.
    pub fn new(gvk: Gvk, variant: FeedVariant) -> Self {
        Self {
            gvk,
            namespace: None,
            variant,
            label_selector: None,
            field_selector: None,
        }
    }

    /// The same feed in `namespace`; an empty name means the whole cluster.
    #[must_use]
    pub fn in_namespace(mut self, namespace: Option<impl Into<String>>) -> Self {
        self.namespace = namespace.map(Into::into).filter(|ns| !ns.is_empty());
        self
    }

    /// Sets the label selector; an empty one means none.
    #[must_use]
    pub fn labels(mut self, selector: impl Into<String>) -> Self {
        self.label_selector = Some(selector.into()).filter(|s| !s.is_empty());
        self
    }

    /// Sets the field selector; an empty one means none.
    #[must_use]
    pub fn fields(mut self, selector: impl Into<String>) -> Self {
        self.field_selector = Some(selector.into()).filter(|s| !s.is_empty());
        self
    }

    /// The same feed carrying `variant`.
    pub(crate) fn with_variant(&self, variant: FeedVariant) -> Self {
        Self {
            variant,
            ..self.clone()
        }
    }

    /// The options of the reflector feed (full or metadata-only) for this request.
    pub(crate) fn watch_options(&self) -> WatchOptions {
        WatchOptions {
            label_selector: self.label_selector.clone(),
            field_selector: self.field_selector.clone(),
            metadata_only: self.variant == FeedVariant::Metadata,
            page_size: None,
        }
    }

    /// The options of the Table feed for this request: rows keyed by their metadata.
    pub(crate) fn table_options(&self) -> TableOptions {
        let list = ListOptions {
            label_selector: self.label_selector.clone(),
            field_selector: self.field_selector.clone(),
            ..ListOptions::default()
        };
        TableOptions::default()
            .list(list)
            .include_object(IncludeObject::Metadata)
    }

    /// `"<gvk> in <namespace>"` or `"<gvk> cluster-wide"`, for messages.
    pub(crate) fn describe(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{} {} in {ns}", self.variant.as_str(), self.gvk),
            None => format!("{} {} cluster-wide", self.variant.as_str(), self.gvk),
        }
    }
}
