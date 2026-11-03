//! The `oxikube` block: colours Zed themes have no keys for.

use gpui::Hsla;

/// How many cluster tab colours a theme provides.
pub const CLUSTER_TAB_COLORS: usize = 8;

/// How many pod colours a theme provides for the multi-pod log view.
pub const LOG_SOURCE_COLORS: usize = 10;

/// Kubernetes status colours and the cluster-tab and log-source palettes.
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
    /// Argo CD health `Healthy` (default: success).
    pub health_healthy: Hsla,
    /// Argo CD health `Progressing` (default: warning).
    pub health_progressing: Hsla,
    /// Argo CD health `Degraded` (default: error).
    pub health_degraded: Hsla,
    /// Argo CD health `Suspended` (default: hint).
    pub health_suspended: Hsla,
    /// Argo CD health `Missing` (default: muted text). Health `Unknown` uses `status_unknown`.
    pub health_missing: Hsla,
    /// Argo CD sync status `Synced` (default: success).
    pub sync_synced: Hsla,
    /// Argo CD sync status `OutOfSync` (default: warning).
    pub sync_out_of_sync: Hsla,
    /// Argo CD sync status `Unknown` (default: muted text).
    pub sync_unknown: Hsla,
    /// A line or resource a diff adds (default: success).
    pub diff_added: Hsla,
    /// A line or resource a diff removes (default: error).
    pub diff_removed: Hsla,
    /// A line or resource a diff changes (default: the modified status colour).
    pub diff_changed: Hsla,
    /// The palette a cluster tab can be coloured with, so production and staging tabs differ at
    /// a glance (default: the theme's player colours).
    pub cluster_tabs: [Hsla; CLUSTER_TAB_COLORS],
    /// The palette the multi-pod log view colours each pod's prefix with: a pod takes the colour
    /// its name hashes to, so it keeps it across reopens (default: the terminal's ANSI blue,
    /// green, yellow, magenta and cyan, then their bright variants; never red, which is the error
    /// level's).
    pub log_sources: [Hsla; LOG_SOURCE_COLORS],
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
            health_healthy: color,
            health_progressing: color,
            health_degraded: color,
            health_suspended: color,
            health_missing: color,
            sync_synced: color,
            sync_out_of_sync: color,
            sync_unknown: color,
            diff_added: color,
            diff_removed: color,
            diff_changed: color,
            cluster_tabs: [color; CLUSTER_TAB_COLORS],
            log_sources: [color; LOG_SOURCE_COLORS],
        }
    }
}
