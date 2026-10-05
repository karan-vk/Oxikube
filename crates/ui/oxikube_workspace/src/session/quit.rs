//! Quitting, and the confirmation while operations run.
//!
//! Features that start long-lived work (exec sessions and port-forwards in E09, applies in E15)
//! register an *operation provider*: a function that lists what is running right now. A quit
//! ([`Quit`], `cmd-q` / `ctrl-q`, the menu) asks every provider; with nothing running, or with
//! `confirm_quit` off, the app quits at once. Otherwise a confirm dialog lists the operations on
//! the window, and only its confirm button quits. The dialog is an overlay on the window's `Root`
//! (E05-S03): nothing waits for the answer, so the UI thread never blocks.

#[cfg(test)]
use std::rc::Rc;

use gpui::{
    App, Global, ParentElement as _, SharedString, Styled as _, Window, WindowId, actions, div,
};
use oxikube_settings::Settings as _;
use oxikube_ui::{
    button::ButtonVariant,
    dialog::{AlertDialog, OverlayExt as _},
    layout::v_flex,
};

use super::settings::SessionSettings;

actions!(
    app,
    [
        /// Quits the application; asks first while operations are running.
        Quit,
    ]
);

/// How many operations the dialog lists before summarising the rest.
const LISTED: usize = 8;

/// One thing that would be lost by quitting: an exec session, a port-forward, a running apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningOperation {
    /// What kind of operation it is (`Exec session`, `Port-forward`).
    pub kind: SharedString,
    /// What it works on (`pod/web-0 in prod`).
    pub label: SharedString,
}

impl RunningOperation {
    /// An operation of `kind` on `label`.
    pub fn new(kind: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            kind: kind.into(),
            label: label.into(),
        }
    }
}

/// Identifies a registered provider, for [`unregister_operation_provider`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationProviderId(u64);

type Provider = Box<dyn Fn(&App) -> Vec<RunningOperation>>;

#[derive(Default)]
struct QuitGuard {
    next_id: u64,
    providers: Vec<(OperationProviderId, Provider)>,
    /// The window our confirm dialog was last opened on, to not stack a second one on it.
    prompt_window: Option<WindowId>,
    /// Replaces `cx.quit()` (the test platform's quit does nothing observable).
    #[cfg(test)]
    quit_hook: Option<Rc<dyn Fn(&mut App)>>,
}

impl Global for QuitGuard {}

/// Registers `provider`, which is asked for the running operations on every quit request. It
/// runs on the UI thread: read in-memory state, never wait on I/O.
pub fn register_operation_provider(
    cx: &mut App,
    provider: impl Fn(&App) -> Vec<RunningOperation> + 'static,
) -> OperationProviderId {
    let guard = cx.default_global::<QuitGuard>();
    let id = OperationProviderId(guard.next_id);
    guard.next_id += 1;
    guard.providers.push((id, Box::new(provider)));
    id
}

/// Removes a provider registered with [`register_operation_provider`].
pub fn unregister_operation_provider(cx: &mut App, id: OperationProviderId) {
    cx.default_global::<QuitGuard>()
        .providers
        .retain(|(provider_id, _)| *provider_id != id);
}

/// Everything the providers report as running now.
pub fn running_operations(cx: &App) -> Vec<RunningOperation> {
    cx.try_global::<QuitGuard>()
        .map(|guard| {
            guard
                .providers
                .iter()
                .flat_map(|(_, provider)| provider(cx))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether quitting now has to ask: operations are running and `confirm_quit` is on (the default
/// when there is no settings store).
pub(super) fn needs_confirmation(cx: &App) -> bool {
    let confirm = SessionSettings::try_get(cx).is_none_or(|settings| settings.confirm_quit);
    confirm && !running_operations(cx).is_empty()
}

/// Quits, or opens the confirm dialog first (see the module docs). Does not wait for the answer.
pub fn request_quit(cx: &mut App) {
    if !needs_confirmation(cx) {
        quit_now(cx);
        return;
    }
    // Action handlers run while the window that dispatched the action is borrowed, so the dialog
    // opens once that update has ended.
    cx.defer(|cx| {
        // On macOS the app outlives its last window, so operations may still be running with no
        // window to ask on: bring a window back for the question rather than quit silently.
        let window = cx
            .active_window()
            .or_else(|| cx.windows().first().copied())
            .or_else(|| crate::window::open_main_window(cx).ok().map(Into::into));
        let shown = window.is_some_and(|window| {
            window
                .update(cx, |_, window, cx| {
                    window.activate_window();
                    show_quit_prompt(window, cx)
                })
                .is_ok()
        });
        if !shown {
            // Not even a window could be opened: nobody to ask.
            quit_now(cx);
        }
    });
}

/// Quits without asking.
pub(super) fn quit_now(cx: &mut App) {
    #[cfg(test)]
    if let Some(hook) = cx
        .try_global::<QuitGuard>()
        .and_then(|guard| guard.quit_hook.clone())
    {
        hook(cx);
        return;
    }
    cx.quit();
}

/// Replaces the quit itself with `hook`.
#[cfg(test)]
pub(super) fn set_quit_hook(cx: &mut App, hook: impl Fn(&mut App) + 'static) {
    cx.default_global::<QuitGuard>().quit_hook = Some(Rc::new(hook));
}

/// Opens the confirm dialog on `window`, unless ours is already open there.
pub(super) fn show_quit_prompt(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    let guard = cx.default_global::<QuitGuard>();
    let open_here = guard.prompt_window == Some(id);
    guard.prompt_window = Some(id);
    if open_here && window.has_active_dialog(cx) {
        return;
    }
    let operations = running_operations(cx);
    window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
        alert
            .confirm()
            .title("Quit Oxikube?")
            .description(describe(&operations))
            .ok_text("Quit")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Keep Running")
            .on_ok(|_, _, cx| {
                cx.default_global::<QuitGuard>().prompt_window = None;
                quit_now(cx);
                true
            })
            .on_cancel(|_, _, cx| {
                cx.default_global::<QuitGuard>().prompt_window = None;
                true
            })
    });
}

/// The dialog's operation lines: one per operation, the first few only.
pub(super) fn operation_lines(operations: &[RunningOperation]) -> Vec<String> {
    let mut lines: Vec<String> = operations
        .iter()
        .take(LISTED)
        .map(|op| format!("{}: {}", op.kind, op.label))
        .collect();
    if operations.len() > LISTED {
        lines.push(format!("and {} more", operations.len() - LISTED));
    }
    lines
}

/// The dialog body: the consequence, then the operations.
fn describe(operations: &[RunningOperation]) -> impl gpui::IntoElement + use<> {
    v_flex()
        .gap_1()
        .child("These are still running and will be stopped:")
        .children(
            operation_lines(operations)
                .into_iter()
                .map(|line| div().child(line)),
        )
}
