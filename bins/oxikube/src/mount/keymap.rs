//! The user's `keymap.json` in the running app (E11-S08).
//!
//! The keymap itself (layers, validation, hot reload) is `oxikube_keymap`'s; it is platform code
//! and can neither show a toast nor open a tab, so the two pieces that need the window are here:
//!
//! - [`follow_diagnostics`]: when the user's file has problems, one persistent toast lists them
//!   with their `keymap.json:<line>` and an "Open keymap" button; fixing the file takes it away.
//! - `keymap::OpenUser` ([`register_commands`], [`open_requests`], [`install_action`]): creates
//!   `keymap.json` from the commented template when it is missing and opens it in the editor.
//!   There is no manifest editor tab yet (E10-S10 opens it in JSON mode when it lands), so the
//!   system's editor for `.json` files opens it.
//!
//! The command is a bus command like any other, so the palette (E11-S03), a key bound to the
//! `keymap::OpenUser` action in `keymap.json`, and the MCP tool all run the same handler.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{AnyWindowHandle, App, Global, Subscription, Task, WeakEntity};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_keymap::{KeymapDiagnosticsEvent, OpenUser};
use oxikube_workspace::{CommandDispatcher, Toast, ToastAction, Workspace};
use serde_json::json;

/// The key of the toast, so a second load with other problems replaces it and a fix removes it.
const TOAST_KEY: &str = "keymap-diagnostics";

/// Where `keymap::OpenUser` sends the file it created or found, to be opened on the UI thread.
pub type OpenSink = mpsc::UnboundedSender<PathBuf>;

/// A channel for [`OpenSink`] and its receiver for [`open_requests`].
pub fn channel() -> (OpenSink, mpsc::UnboundedReceiver<PathBuf>) {
    mpsc::unbounded()
}

/// `keymap::OpenUser`: creates `file` from the template when it does not exist (off the UI
/// thread: the bus runs handlers elsewhere), then asks the window to open it. `file` is the keymap's
/// own path (`oxikube_keymap::user_keymap_file`); `None` when the app runs without a config
/// directory, which the command reports instead of doing nothing.
///
/// # Errors
///
/// A [`RegisterError`] when the command is not declared or already has a handler.
pub fn register_commands(
    registry: &mut CommandRegistry,
    file: Option<PathBuf>,
    open: OpenSink,
) -> Result<(), RegisterError> {
    let meta = command::lookup(CommandId::KEYMAP_OPEN_USER)
        .copied()
        .ok_or(RegisterError::Undeclared(CommandId::KEYMAP_OPEN_USER))?;
    registry.register(meta, move |_: Command, _: HandlerContext| {
        let file = file.clone();
        let open = open.clone();
        async move {
            let Some(file) = file else {
                return Err(OxiError::not_found(
                    "this session has no config directory, so there is no keymap.json to open",
                ));
            };
            // One small file, on the bus's worker (never the UI thread).
            let (path, created) = create_if_missing(&file)?;
            open.unbounded_send(path.clone())
                .map_err(|_| OxiError::internal("the main window is gone"))?;
            Ok(CommandOutput::data(
                json!({ "path": path.display().to_string(), "created": created }),
            ))
        }
    })
}

/// Make sure `file` exists, creating its directory and the commented template when it does not.
/// Returns the path and whether it was created. An existing file is never touched.
fn create_if_missing(file: &Path) -> Result<(PathBuf, bool), OxiError> {
    let existed = file.exists();
    let dir = file
        .parent()
        .ok_or_else(|| OxiError::internal("the keymap file has no directory"))?;
    let path = oxikube_keymap::ensure_user_keymap(dir)?;
    Ok((path, !existed))
}

/// How `keymap.json` is shown to the user: the system's editor for the file unless a global of
/// this type says otherwise. The seam where the manifest editor in JSON mode (E10-S10) replaces
/// the system editor, and where tests see the file without launching one.
#[derive(Clone)]
pub struct KeymapFileOpener(pub Rc<dyn Fn(&Path, &mut App)>);

impl Global for KeymapFileOpener {}

/// Opens each file `keymap::OpenUser` prepared, on the UI thread.
pub fn open_requests(mut files: mpsc::UnboundedReceiver<PathBuf>, cx: &mut App) -> Task<()> {
    cx.spawn(async move |cx| {
        while let Some(path) = files.next().await {
            cx.update(|cx| match cx.try_global::<KeymapFileOpener>().cloned() {
                Some(opener) => (opener.0)(&path, cx),
                None => cx.open_with_system(&path),
            });
        }
    })
}

/// Runs the `keymap::OpenUser` command when a key bound to the action is pressed in `window`.
///
/// The action is app-wide (GPUI offers it to the app after the focused view declined it), and
/// every window installs this once, so each only answers while its own window is the active one.
/// The handler lives as long as the app, so it holds the window's dispatcher weakly: a closed
/// window's views and bus are not kept alive by it.
pub fn install_action(
    window: AnyWindowHandle,
    dispatcher: &Rc<dyn CommandDispatcher>,
    cx: &mut App,
) {
    let dispatcher = Rc::downgrade(dispatcher);
    cx.on_action(move |_: &OpenUser, cx| {
        if cx.active_window() == Some(window)
            && let Some(dispatcher) = dispatcher.upgrade()
        {
            dispatcher.dispatch(Command::KeymapOpenUser, cx);
        }
    });
}

/// Shows the toast for the problems in the user's `keymap.json` in `workspace`, now (the problems
/// the start-up load found) and whenever the list changes. Keep the subscription as long as the
/// window lives.
pub fn follow_diagnostics(
    workspace: WeakEntity<Workspace>,
    dispatcher: Rc<dyn CommandDispatcher>,
    cx: &mut App,
) -> Subscription {
    let show = move |event: &KeymapDiagnosticsEvent, cx: &mut App| {
        let Some(workspace) = workspace.upgrade() else {
            return;
        };
        if event.is_clear() {
            let layer = workspace.read(cx).toast_layer().clone();
            layer.update(cx, |layer, cx| layer.dismiss_key(TOAST_KEY, cx));
            return;
        }
        let dispatcher = dispatcher.clone();
        let toast = Toast::warning(event.message())
            .key(TOAST_KEY)
            .persistent()
            .action(ToastAction::new("Open keymap", move |_, cx| {
                dispatcher.dispatch(Command::KeymapOpenUser, cx);
            }));
        workspace.update(cx, |workspace, cx| workspace.show_toast(toast, cx));
    };
    let subscription = oxikube_keymap::subscribe_diagnostics(cx, show.clone());
    // The load at start-up happened before this window existed.
    let current = oxikube_keymap::user_diagnostics(cx);
    if !current.is_empty() {
        show(
            &KeymapDiagnosticsEvent {
                diagnostics: current,
            },
            cx,
        );
    }
    subscription
}
