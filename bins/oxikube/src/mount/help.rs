//! The help overlay of the main window (E11-S10).
//!
//! `?` is bound to `help::Show` in the keymap files (inside a cluster tab, nowhere a text field
//! has the focus); the `oxikube_palette` action opens the overlay of the window that has the
//! focus, and the same request arrives from the bus for the command palette and agents. The
//! overlay reads the keymap and the focused view's key contexts when it opens, so it needs nothing
//! from the app's services.

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, Window};
use oxikube_palette::help::HelpHost;
use oxikube_workspace::Workspace;
use std::rc::Rc;

/// Installs the help overlay of `window` and starts serving the bus's `help::Show` requests. The
/// returned task lives as long as the window.
pub fn mount(
    workspace: &Entity<Workspace>,
    requests: UnboundedReceiver<()>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let host = Rc::new(HelpHost::new(workspace));
    host.install(window, cx);
    host.serve(requests, window, cx)
}
