//! Metadata-only feeds (E04-S03): the cheap list view, and upgrading one row to a full object.
//!
//! A metadata feed is the reflector feed of [`reflector_feed`](KubeResources::reflector_feed)
//! with `WatchOptions::metadata_only`: the same watcher, reflector store, relist diff,
//! coalescing, backoff and [`ReflectorFeed`] handle, but the list and watch requests ask the
//! server for `PartialObjectMetadata` (kube's `Api<PartialObjectMeta<_>>`, sent with
//! `Accept: application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1`). The server then
//! leaves out `spec`, `status` and data, which is most of the bytes of a Pod and all of a
//! ConfigMap or Secret. What arrives is a [`Resource`] with `apiVersion`, `kind` and `metadata`
//! (name, namespace, uid, labels, annotations, owner references, timestamps, resource
//! version) and [`Resource::is_partial`] set, so a view cannot mistake it for a complete
//! object.
//!
//! # Which kinds benefit
//!
//! Kinds whose objects are large next to their metadata, and which a table shows from
//! metadata alone or with little else: Pods (the 10 k-pod case), ReplicaSets, Events,
//! ConfigMaps and Secrets (a metadata feed never carries Secret data), Nodes' bulky status,
//! and the large list kinds of CRD-heavy clusters. Columns that read `spec` or `status`
//! (Ready, Restarts, Node, IP) need the full feed or the Table feed. Which one a kind gets is
//! the watch budget's decision (E04-S13) with the `ColumnProvider`s; this module only
//! provides the metadata variant.
//!
//! # Upgrading on demand
//!
//! [`KubeResources::upgrade`] is one `GET` of the object a partial row stands for, returned as
//! a complete [`Resource`]. It does not touch any running feed (it is not a second feed per
//! row, and no watch request is made), so the feed keeps delivering partial objects and the
//! app decides where the upgraded one is shown.

use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ResourceReader, WatchOptions};

use super::ReflectorFeed;
use crate::resources::KubeResources;

impl KubeResources {
    /// Opens a metadata-only feed of `kind` over `scope`: [`reflector_feed`] with
    /// `options.metadata_only` set.
    ///
    /// # Errors
    ///
    /// As [`reflector_feed`].
    ///
    /// [`reflector_feed`]: KubeResources::reflector_feed
    pub async fn metadata_feed(
        &self,
        kind: &Gvk,
        scope: &WatchScope,
        options: &WatchOptions,
    ) -> OxiResult<ReflectorFeed> {
        let mut options = options.clone();
        options.metadata_only = true;
        self.reflector_feed(kind, scope, &options).await
    }

    /// The complete object behind `partial`: one `GET` by its kind, namespace and name.
    ///
    /// `partial` is typically a row of a metadata feed, but any resource works. Does not
    /// affect running feeds.
    ///
    /// # Errors
    ///
    /// `NotFound` when the object is gone, or when the name now belongs to a different
    /// object (its UID changed: it was deleted and re-created, and the feed will say so
    /// shortly). Otherwise as [`ResourceReader::get`].
    pub async fn upgrade(&self, partial: &Resource) -> OxiResult<Resource> {
        let full = self
            .get(&partial.kind, partial.namespace(), partial.name())
            .await?;
        match (&partial.meta.uid, &full.meta.uid) {
            (Some(old), Some(new)) if old != new => Err(OxiError::not_found(format!(
                "{} {} was replaced by a new object",
                partial.kind,
                partial.name()
            ))),
            _ => Ok(full),
        }
    }
}
