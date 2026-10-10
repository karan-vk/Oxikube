//! [`JumpEnv`]: what a jump line is resolved against.
//!
//! The parser knows only syntax. Turning `deploy kube-system @prod` into navigation needs the
//! alias table of the cluster, the namespaces it has and the cluster contexts the user can
//! switch to. The binary implements this over the live app state; tests implement it over
//! plain data, so the planner needs no window and no cluster.

use std::sync::Arc;

use oxikube_domain::ids::ClusterId;

use crate::search::aliases::AliasTable;
use crate::session::namespaces::NamespaceCatalog;

/// One cluster context the user can switch to with `:ctx name` or `@name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpContext {
    /// The kubeconfig context name, which is what the user types.
    pub name: Arc<str>,
    /// The cluster it is.
    pub cluster: ClusterId,
    /// Whether the cluster has a live session (its tab can be shown at once); otherwise
    /// switching to it connects first.
    pub connected: bool,
}

/// The facts a jump line is resolved against. All reads are synchronous and cheap: the caller
/// keeps the namespace lists and contexts it has already loaded.
pub trait JumpEnv {
    /// The cluster whose tab is shown (where a line without `@ctx` acts), if any.
    fn active_cluster(&self) -> Option<ClusterId>;

    /// Every cluster context, in the order they are offered.
    fn contexts(&self) -> &[JumpContext];

    /// The alias table of `cluster` (built-in, discovered and the user's aliases).
    fn aliases(&self, cluster: &ClusterId) -> AliasTable;

    /// The namespaces `cluster` has, when they are known yet. A catalog that is not the
    /// cluster's own list (RBAC forbade it, the list failed) accepts any valid name.
    fn namespaces(&self, cluster: &ClusterId) -> Option<NamespaceCatalog>;
}
