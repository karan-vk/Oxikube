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
/// `cargo xtask lint-deps` scans every `ui`, `platform` and `bins` crate's non-test sources
/// (`xtask/src/lint_mutation`) and fails on any mention of this type, on holding a
/// `ResourceWriter` (implementing one is allowed), and on mutating calls on a writer handle.
/// There is no allow-list.
///
/// Outside this crate the constructor is unreachable (the doctest below documents it; this
/// crate sets `doctest = false`, so the `constructor_stays_crate_private` unit test is what
/// actually guards it):
///
/// ```compile_fail,E0624
/// use std::sync::Arc;
/// use oxikube_app::guard::Mutation;
/// fn forge(writer: Arc<dyn oxikube_ports::ResourceWriter>, cluster: oxikube_domain::ids::ClusterId) {
///     let _ = Mutation::new(cluster, writer, false);
/// }
/// ```
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

    /// The cluster's writer as a shared handle, for work that outlives a borrow but not the
    /// handler (the drain stream the `DrainPort` returns). Crate-private: it stays inside the
    /// handler of the command the guard admitted, like [`writer`](Self::writer).
    pub(crate) fn writer_handle(&self) -> Arc<dyn ResourceWriter> {
        self.writer.clone()
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

#[cfg(test)]
mod tests {
    /// The crate has `doctest = false`, so the `compile_fail` doctest never runs. This guards the
    /// same two facts from source: the constructor is crate-private and the token is not `Clone`.
    #[test]
    fn constructor_stays_crate_private() {
        let src = include_str!("mutation.rs");
        // The needles are built from pieces so this test does not match its own text.
        let private_ctor = concat!("pub(crate)", " fn new(");
        let public_ctor = concat!("pub", " fn new(");
        assert!(
            src.contains(private_ctor),
            "Mutation::new must be pub(crate)"
        );
        assert!(!src.contains(public_ctor), "Mutation::new must not be pub");
        let decl = concat!("pub struct", " Mutation {");
        let before = &src[..src.find(decl).expect("Mutation declaration")];
        let attrs = before.rsplit("\n\n").next().unwrap_or_default();
        assert!(
            !attrs.contains("derive("),
            "Mutation must not derive Clone, Default or anything else"
        );
        assert!(
            !src.contains(concat!("impl Clone", " for Mutation")),
            "Mutation must not be Clone"
        );
    }
}
