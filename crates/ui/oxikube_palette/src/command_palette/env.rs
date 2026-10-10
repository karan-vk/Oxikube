//! [`PaletteEnv`]: what the palette needs to know about the running app, and nothing else.

use gpui::App;
use oxikube_app::ActionContext;
use oxikube_domain::ids::ClusterId;

/// The app facts a palette asks when it opens: which cluster is shown and what its session
/// allows. A trait so the palette crate depends on no session manager, and tests script it.
pub trait PaletteEnv: 'static {
    /// The cluster whose tab is shown, `None` on the catalog or an empty workspace.
    fn active_cluster(&self, cx: &App) -> Option<ClusterId>;

    /// What the session of `cluster` allows (capabilities, read-only), `None` when it has no
    /// live session.
    fn session(&self, cluster: &ClusterId, cx: &App) -> Option<ActionContext>;
}
