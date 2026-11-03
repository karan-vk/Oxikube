//! The theme picker of the main window (E22-S08).
//!
//! "Select Theme" in the command palette (and the `theme_select` tool for agents) runs the
//! `theme::Select` command; this opens the picker over the window's modal layer. Moving the
//! selection previews a theme, Enter writes the `theme` setting, Escape or a click outside puts
//! the old theme back.

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, Window};
use oxikube_palette::theme_picker::ThemeHost;
use oxikube_workspace::Workspace;
use std::rc::Rc;

/// Starts serving the bus's `theme::Select` requests for `window`. The returned task lives as
/// long as the window.
pub fn mount(
    workspace: &Entity<Workspace>,
    requests: UnboundedReceiver<()>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    Rc::new(ThemeHost::new(workspace)).serve(requests, window, cx)
}
