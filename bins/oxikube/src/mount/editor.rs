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
//! E10-S09 adds create from template: `resource::Create` (the palette's "Create Resource…",
//! `cmd-alt-n` / `ctrl-alt-n`, the "New" button of every resource table) opens the "Create
//! resource" picker over the embedded manifests of the core kinds and the user's
//! `<config dir>/templates/*.yaml` ([`templates`], read through the filesystem port), and a
//! buffer of several documents is applied one object at a time.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc;
use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::{ApplyService, ClusterSessionManager, TemplateService};
use oxikube_editor::view::{ClusterEditorHost, EditorRequest, EditorViews, EditorViewsDeps};
use oxikube_ports::FsPort;
use oxikube_workspace::{ClusterTabs, CommandDispatcher, Workspace};

use crate::aliases::ConfigSourceGlobal;
use crate::startup::ConfigSource;

/// Starts the window's editor views over `requests` (the bus's `editor::*` handlers).
pub fn start_views(
    sessions: ClusterSessionManager,
    apply: ApplyService,
    templates: Option<TemplateService>,
    tabs: WeakEntity<ClusterTabs>,
    workspace: &Entity<Workspace>,
    dispatcher: Rc<dyn CommandDispatcher>,
    requests: mpsc::UnboundedReceiver<EditorRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<EditorViews> {
    oxikube_editor::view::install(dispatcher.clone(), cx);
    let mut host = ClusterEditorHost::new(tabs, sessions).with_apply(apply);
    if let Some(templates) = templates {
        host = host.with_templates(templates);
    }
    let deps = EditorViewsDeps {
        host: Rc::new(host),
        window_workspace: workspace.downgrade(),
        dispatcher,
    };
    EditorViews::start(deps, requests, window, cx)
}

/// The "Create resource" templates (E10-S09): the editor's embedded manifests plus the
/// `templates` folder of the config directory the app was started with (the same
/// [`ConfigSource`] as `aliases.json`), read through `fs` each time the picker opens. Only the
/// embedded ones are offered without a config directory (in-memory config, or no start-up).
pub fn templates(fs: Arc<dyn FsPort>, cx: &App) -> Option<TemplateService> {
    let source = cx.try_global::<ConfigSourceGlobal>().map(|g| &g.0);
    let dir = templates_dir(source)?;
    Some(TemplateService::new(
        oxikube_editor::templates::embedded(),
        fs,
        dir,
    ))
}

/// `<config dir>/templates` for where the app reads its config from.
fn templates_dir(source: Option<&ConfigSource>) -> Option<PathBuf> {
    let dir = match source? {
        ConfigSource::UserDir => oxikube_settings::paths::config_dir()?,
        ConfigSource::Dir(dir) => dir.clone(),
        ConfigSource::Memory => return None,
    };
    Some(dir.join("templates"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    #[test]
    fn the_templates_folder_follows_the_startup_config_source() {
        let dir = PathBuf::from("/tmp/oxikube-test-config");
        assert_eq!(
            templates_dir(Some(&ConfigSource::Dir(dir))).as_deref(),
            Some(Path::new("/tmp/oxikube-test-config/templates"))
        );
        assert_eq!(templates_dir(Some(&ConfigSource::Memory)), None);
        assert_eq!(templates_dir(None), None, "never the real home directory");
    }
}
