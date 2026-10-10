//! The manifest editors of the main window (E10-S04).
//!
//! [`start_views`] starts the window's [`EditorViews`], which apply `editor::NewManifest` (an
//! empty editor in the shown cluster's tab, its YAML checked against that cluster's schemas
//! through the session's `SchemaPort`; a tab of the window with syntax checks only when no
//! cluster tab is shown) and the focused editor's `editor::ToggleReadOnly` /
//! `editor::ToggleSoftWrap`. The keymap's `editor::NewManifest` reaches the bus through the
//! dispatcher installed here.

use std::rc::Rc;

use futures::channel::mpsc;
use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_editor::view::{ClusterEditorHost, EditorRequest, EditorViews, EditorViewsDeps};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, Workspace};

/// Starts the window's editor views over `requests` (the bus's `editor::*` handlers).
pub fn start_views(
    sessions: ClusterSessionManager,
    tabs: WeakEntity<ClusterTabs>,
    workspace: &Entity<Workspace>,
    dispatcher: Rc<dyn CommandDispatcher>,
    requests: mpsc::UnboundedReceiver<EditorRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<EditorViews> {
    oxikube_editor::view::install(dispatcher.clone(), cx);
    let deps = EditorViewsDeps {
        host: Rc::new(ClusterEditorHost::new(tabs, sessions)),
        window_workspace: workspace.downgrade(),
        dispatcher,
    };
    EditorViews::start(deps, requests, window, cx)
}
