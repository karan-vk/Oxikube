//! The window's side of the `theme` setting (E22-S07): following the operating system's
//! light/dark switch and the toasts for a theme name that is not installed (S07) and for a theme
//! file that could not be loaded (S09).
//!
//! `oxikube_theme` is platform code: it resolves the setting and announces problems but can
//! neither observe a window nor show a toast, so both pieces are mounted here.
//!
//! **Multi-window policy.** GPUI reports appearance per window, but the theme is one for the
//! app. The first window to mount owns the appearance (it calls
//! [`SystemAppearance::follow`] and keeps the subscription); later windows do not, so two windows
//! never fight over the global. When the owner closes, the next window to mount takes over.

use gpui::{AnyWindowHandle, App, Global, Subscription, Window};
use oxikube_theme::{
    SystemAppearance, ThemeNotice, pending_file_problems, pending_unknown_theme, subscribe_notices,
};
use oxikube_workspace::{Toast, Workspace};

/// The window that currently follows the operating system's appearance.
struct AppearanceOwner(AnyWindowHandle);

impl Global for AppearanceOwner {}

/// What a window keeps alive for the theme setting.
pub struct ThemeFollow {
    _appearance: Option<Subscription>,
    _notices: Subscription,
}

/// Mounts the appearance follower (when this window is the owner) and the theme toasts.
pub fn mount(
    workspace: gpui::WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> ThemeFollow {
    let handle = window.window_handle();
    let owner_alive = cx
        .try_global::<AppearanceOwner>()
        .is_some_and(|owner| owner.0 != handle && cx.windows().contains(&owner.0));
    let appearance = if owner_alive {
        None
    } else {
        cx.set_global(AppearanceOwner(handle));
        Some(SystemAppearance::follow(window, cx))
    };

    let show = move |notice: &ThemeNotice, cx: &mut App| {
        let Some(workspace) = workspace.upgrade() else {
            return;
        };
        let toast = match notice {
            ThemeNotice::UnknownTheme { .. } => Toast::warning(notice.message()),
            ThemeNotice::BadThemeFile { .. } => Toast::error(notice.message()),
        }
        .key(notice.key());
        workspace.update(cx, |workspace, cx| workspace.show_toast(toast, cx));
    };
    let notices = subscribe_notices(cx, show.clone());
    // The start-up resolution happened before this window existed.
    if let Some(notice) = pending_unknown_theme(cx) {
        show(&notice, cx);
    }
    for notice in pending_file_problems(cx) {
        show(&notice, cx);
    }
    ThemeFollow {
        _appearance: appearance,
        _notices: notices,
    }
}
