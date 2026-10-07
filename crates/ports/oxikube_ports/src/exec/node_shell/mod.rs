//! A shell on a node through a privileged helper pod: the template ([`NodeShellSpec`]) the user's
//! settings fill in, and the pod and command it renders to (E09-S09).
//!
//! The template is plain data in the ports crate because three layers read it: the app builds it
//! from the cluster's settings and shows its image and namespace in the confirmation, the guard's
//! handler dry-runs [`node_shell_manifest`] against the server before anything is created, and the
//! adapter creates exactly that manifest and runs [`node_shell_command`] in it.
//!
//! | Piece | Where |
//! |---|---|
//! | [`NodeShellSpec`], [`NodeShellToleration`], the defaults | `spec` |
//! | [`node_shell_manifest`], [`node_shell_command`], the label names | `manifest` |

mod manifest;
mod spec;

pub use manifest::{
    CONTAINER_NAME, HEARTBEAT_ANNOTATION, NODE_ANNOTATION, NODE_SHELL_LABEL, node_shell_command,
    node_shell_manifest,
};
pub use spec::{
    DEFAULT_NODE_SHELL_IMAGE, DEFAULT_NODE_SHELL_NAMESPACE, DEFAULT_NSENTER_ARGS, NodeShellSpec,
    NodeShellToleration,
};
