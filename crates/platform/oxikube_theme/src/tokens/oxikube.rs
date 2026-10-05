//! The `oxikube` block: colours Zed themes have no keys for.

use gpui::Hsla;

/// How many cluster tab colours a theme provides.
pub const CLUSTER_TAB_COLORS: usize = 8;

/// Kubernetes status colours and the cluster-tab palette.
///
/// A theme file may carry an optional `oxikube` object next to `style` (keys listed in
/// `import::table`); anything it leaves out is derived from the theme's own status colours, so
/// every Zed theme gets sensible Kubernetes colours without knowing about them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OxikubeColors {
    /// Pod `Running`, node `Ready`, workload available (default: success).
    pub status_running: Hsla,
    /// `Pending`, `ContainerCreating`, rolling out (default: warning).
    pub status_pending: Hsla,
    /// `Failed`, `CrashLoopBackOff`, `ImagePullBackOff`, `Error` (default: error).
    pub status_failed: Hsla,
    /// `Succeeded` / `Completed` (default: info).
    pub status_succeeded: Hsla,
    /// `Terminating` (default: hidden).
    pub status_terminating: Hsla,
    /// `Unknown` and anything unclassified (default: muted text).
    pub status_unknown: Hsla,
    /// The palette a cluster tab can be coloured with, so production and staging tabs differ at
    /// a glance (default: the theme's player colours).
    pub cluster_tabs: [Hsla; CLUSTER_TAB_COLORS],
}

impl OxikubeColors {
    pub(crate) fn splat(color: Hsla) -> Self {
        Self {
            status_running: color,
            status_pending: color,
            status_failed: color,
            status_succeeded: color,
            status_terminating: color,
            status_unknown: color,
            cluster_tabs: [color; CLUSTER_TAB_COLORS],
        }
    }
}
