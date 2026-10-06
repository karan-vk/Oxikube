//! [`Mutation`]: the only door to a cluster's `ResourceWriter` (ADR 0012).

use std::fmt;
use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use oxikube_ports::{DeleteOptions, ResourceWriter, WriteOptions};

/// Permission to write to one cluster, for the duration of one guarded command.
///
/// Only [`MutationGuard`](super::MutationGuard) can create one (the constructor is
/// crate-private and the type is neither `Clone` nor `Default`), and it does so only
/// after the read-only check, the confirmation and the audit precondition passed. A
/// handler of a mutating command finds it in
/// [`HandlerContext::mutation`](crate::command_bus::HandlerContext::mutation); handlers
/// of read commands never get one. Nothing else in the app exposes a `ResourceWriter`:
/// [`ClusterSession::resources`](crate::ClusterSession::resources) is read-only.
///
/// A lint that rejects `ResourceWriter` calls outside handlers is an `xtask` follow-up;
/// until then this type and review are the enforcement.
pub struct Mutation {
    cluster: ClusterId,
    writer: Arc<dyn ResourceWriter>,
    dry_run: bool,
}

impl Mutation {
    /// Built by the guard only.
    pub(crate) fn new(cluster: ClusterId, writer: Arc<dyn ResourceWriter>, dry_run: bool) -> Self {
        Self {
            cluster,
            writer,
            dry_run,
        }
    }

    /// The cluster this permission is for.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// The cluster's writer. Pass [`write_options`](Self::write_options) or
    /// [`delete_options`](Self::delete_options) so a dry-run dispatch stays a dry run.
    pub fn writer(&self) -> &dyn ResourceWriter {
        self.writer.as_ref()
    }

    /// Whether the dispatch asked for a server-side dry run.
    pub fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// Write options honouring [`dry_run`](Self::dry_run).
    pub fn write_options(&self) -> WriteOptions {
        WriteOptions {
            dry_run: self.dry_run,
            ..WriteOptions::default()
        }
    }

    /// Delete options honouring [`dry_run`](Self::dry_run).
    pub fn delete_options(&self) -> DeleteOptions {
        DeleteOptions {
            dry_run: self.dry_run,
            ..DeleteOptions::default()
        }
    }
}

impl fmt::Debug for Mutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mutation")
            .field("cluster", &self.cluster)
            .field("dry_run", &self.dry_run)
            .finish_non_exhaustive()
    }
}
