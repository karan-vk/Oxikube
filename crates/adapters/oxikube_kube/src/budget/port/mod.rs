//! [`BudgetedResources`]: a connection's resource and Table ports with every feed opened
//! through its [`FeedRegistry`] (E04-F543).
//!
//! The connector hands this out as the connection's `ResourcePort` and `TableFeedPort`, so every
//! feed the app opens through the ports (tables, sidebar counts and detail views through the
//! resource store, log targets through the log service) is admitted, counted and torn down by
//! the cluster's watch budget. Reads that are not feeds (list, get, scale, subresources) and
//! every write go straight to [`KubeResources`].
//!
//! Each `watch` / `table_feed` call is an owned feed ([`FeedRegistry::open_owned`]): the
//! request named in the counters is the kind, namespace, variant and selectors; the stream is
//! opened with the caller's exact options over a byte-counting client.

mod reader;
mod writer;

use oxikube_domain::OxiError;
use oxikube_domain::ids::Gvk;
use oxikube_ports::{FeedVariant, TableOptions, WatchOptions};

use super::registry::FeedRegistry;
use super::request::FeedRequest;
use crate::resources::KubeResources;

/// The data-plane ports of one connection behind its watch budget. See the
/// [module docs](self).
///
/// Cheap to clone; clones share the client and the registry.
#[derive(Clone)]
pub struct BudgetedResources {
    resources: KubeResources,
    feeds: FeedRegistry,
}

impl BudgetedResources {
    /// `resources` with its feeds opened through `feeds`.
    pub fn new(resources: KubeResources, feeds: FeedRegistry) -> Self {
        Self { resources, feeds }
    }

    /// The watch budget the feeds go through.
    pub fn feeds(&self) -> &FeedRegistry {
        &self.feeds
    }
}

impl std::fmt::Debug for BudgetedResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BudgetedResources")
            .field("cluster", self.feeds.cluster())
            .finish_non_exhaustive()
    }
}

/// The budget's name for a `watch` of `kind` in `namespace` with `options`.
fn watch_request(kind: &Gvk, namespace: Option<&str>, options: &WatchOptions) -> FeedRequest {
    let variant = if options.metadata_only {
        FeedVariant::Metadata
    } else {
        FeedVariant::Full
    };
    selectors(
        FeedRequest::new(kind.clone(), variant).in_namespace(namespace),
        options.label_selector.as_deref(),
        options.field_selector.as_deref(),
    )
}

/// The budget's name for a `table_feed` of `kind` in `namespace` with `options`.
fn table_request(kind: &Gvk, namespace: Option<&str>, options: &TableOptions) -> FeedRequest {
    selectors(
        FeedRequest::new(kind.clone(), FeedVariant::Table).in_namespace(namespace),
        options.list.label_selector.as_deref(),
        options.list.field_selector.as_deref(),
    )
}

fn selectors(request: FeedRequest, labels: Option<&str>, fields: Option<&str>) -> FeedRequest {
    request
        .labels(labels.unwrap_or_default())
        .fields(fields.unwrap_or_default())
}

/// The registry handed back a stream of the other shape: a bug in the opener.
fn wrong_shape() -> OxiError {
    OxiError::internal("the watch budget returned a feed of the wrong shape")
}
