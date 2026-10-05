//! [`FeedSource`]: how the registry opens one feed, and the [`FeedStream`] it gets back.
//!
//! [`KubeResources`] is the real source: a full or metadata request opens a reflector feed
//! (E04-S02 / E04-S03) on one namespace or the whole cluster, a Table request the Table API
//! feed (E04-S04), each over a client that counts its bytes. Tests plug in fake sources.

use async_trait::async_trait;
use oxikube_domain::session::WatchScope;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{FeedVariant, TableFeed, WatchFeed};

use super::counters::ByteCounter;
use super::request::FeedRequest;
use crate::resources::KubeResources;

/// A live feed of either shape, as the ports define them.
pub enum FeedStream {
    /// Resource deltas: a full or metadata-only reflector feed.
    Resources(WatchFeed<Resource>),
    /// Table rows.
    Table(TableFeed),
}

impl FeedStream {
    /// The resource feed, if this is one.
    pub fn into_resources(self) -> Option<WatchFeed<Resource>> {
        match self {
            FeedStream::Resources(feed) => Some(feed),
            FeedStream::Table(_) => None,
        }
    }

    /// The Table feed, if this is one.
    pub fn into_table(self) -> Option<TableFeed> {
        match self {
            FeedStream::Table(feed) => Some(feed),
            FeedStream::Resources(_) => None,
        }
    }
}

impl std::fmt::Debug for FeedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FeedStream::Resources(_) => "FeedStream::Resources",
            FeedStream::Table(_) => "FeedStream::Table",
        })
    }
}

/// Opens single-scope feeds for a [`FeedRegistry`](super::FeedRegistry).
#[async_trait]
pub trait FeedSource: Send + Sync + 'static {
    /// Opens the feed `request` describes, counting its received bytes into `bytes`.
    ///
    /// The stream must stop its watches when dropped (abort on drop).
    ///
    /// # Errors
    ///
    /// The feed constructors' errors (`Unsupported` kind, `Validation` scope, transport
    /// failures of a first list), already mapped to `OxiError`.
    async fn open(&self, request: &FeedRequest, bytes: ByteCounter) -> OxiResult<FeedStream>;
}

#[async_trait]
impl FeedSource for KubeResources {
    async fn open(&self, request: &FeedRequest, bytes: ByteCounter) -> OxiResult<FeedStream> {
        let counted = self.counting_bytes(bytes);
        let gvk = &request.gvk;
        let namespace = request.namespace.as_deref();
        if request.variant == FeedVariant::Table {
            let options = request.table_options();
            return Ok(FeedStream::Table(
                counted.open_table_feed(gvk, namespace, &options).await?,
            ));
        }
        let scope = match namespace {
            Some(ns) => WatchScope::Namespaces(vec![ns.to_owned()]),
            None => WatchScope::Cluster,
        };
        let feed = counted
            .reflector_feed(gvk, &scope, &request.watch_options())
            .await?;
        Ok(FeedStream::Resources(feed.into_watch_feed()))
    }
}
