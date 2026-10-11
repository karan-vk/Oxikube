//! The manifest editors of the main window (E10-S04).
//!
//! [`start_views`] starts the window's [`EditorViews`], which apply `editor::NewManifest` (an
//! empty editor in the shown cluster's tab, its YAML checked against that cluster's schemas
//! through the session's `SchemaPort`; a tab of the window with syntax checks only when no
//! cluster tab is shown) and the focused editor's `editor::ToggleReadOnly` /
//! `editor::ToggleSoftWrap` / `editor::TriggerCompletion` / `editor::ShowHover` (E10-S05); E10-S06 adds `resource::Edit` / `resource::ViewYamlInEditor`
//! (an editor on a freshly read object in the object's cluster tab, through the session's
//! `ResourceReader`) and the toggles for `managedFields` and Secret values; E10-S10 the conversions
//! between YAML and JSON and the switch of the language a buffer is read as; E10-S07 the diff vs live
//! commands (the editors of a cluster tab read and watch the edited object through the session's
//! resource port). The keymap's `editor::NewManifest` reaches the bus through the dispatcher installed here.
//! E10-S08 gives the editors of a cluster tab the app's [`ApplyService`]: `resource::Apply`
//! (`cmd-s` / `ctrl-s`, the palette, the toolbar's Apply) dry-runs the buffer, shows the diff and
//! applies it through the bus.

use std::rc::Rc;

use futures::channel::mpsc;
use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::{ApplyService, ClusterSessionManager};
use oxikube_editor::view::{ClusterEditorHost, EditorRequest, EditorViews, EditorViewsDeps};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, Workspace};

/// Starts the window's editor views over `requests` (the bus's `editor::*` handlers).
pub fn start_views(
    sessions: ClusterSessionManager,
    apply: ApplyService,
    tabs: WeakEntity<ClusterTabs>,
    workspace: &Entity<Workspace>,
    dispatcher: Rc<dyn CommandDispatcher>,
    requests: mpsc::UnboundedReceiver<EditorRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<EditorViews> {
    oxikube_editor::view::install(dispatcher.clone(), cx);
    let deps = EditorViewsDeps {
        host: Rc::new(ClusterEditorHost::new(tabs, sessions).with_apply(apply)),
        window_workspace: workspace.downgrade(),
        dispatcher,
    };
    EditorViews::start(deps, requests, window, cx)
}
