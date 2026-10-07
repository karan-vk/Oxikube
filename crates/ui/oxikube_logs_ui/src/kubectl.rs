//! [`follow_kubectl`]: keeps the app's answer to "is kubectl installed?" fresh, off the UI thread.
//!
//! The log view's "Tail in terminal" action is offered only when kubectl is found (E08-S08), and
//! the toolbar reads a cached answer ([`Kubectl`]). The lookup is a few `stat` calls over `PATH`,
//! but it still never runs on the UI thread: it runs on a background task at start-up, when the
//! settings change (a user who just installed kubectl is editing them), and every
//! [`KUBECTL_POLL`] after, so installing kubectl while the app runs shows the action without a
//! restart.

use std::time::Duration;

use gpui::{App, AppContext as _, Subscription, Task};
use oxikube_app::logs::kubectl::Kubectl;
use oxikube_settings::Settings as _;

use crate::LogsSettings;

/// How often the lookup runs again while the app is open.
pub const KUBECTL_POLL: Duration = Duration::from_secs(60);

/// What keeps [`follow_kubectl`] going: dropping it stops the polling and the settings hook.
pub struct KubectlFollow {
    _poll: Task<()>,
    _settings: Subscription,
}

/// Looks for kubectl now, on every change of the settings and every [`KUBECTL_POLL`], each time on a
/// background task. Keep the returned guard for as long as the answer should stay fresh (the
/// app's lifetime).
pub fn follow_kubectl(kubectl: &Kubectl, cx: &mut App) -> KubectlFollow {
    look(kubectl, cx);
    let on_change = kubectl.clone();
    let settings = LogsSettings::observe(cx, move |cx| look(&on_change, cx));
    let polled = kubectl.clone();
    let poll = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(KUBECTL_POLL).await;
            cx.update(|cx| look(&polled, cx));
        }
    });
    KubectlFollow {
        _poll: poll,
        _settings: settings,
    }
}

/// One lookup on a background task.
fn look(kubectl: &Kubectl, cx: &App) {
    let kubectl = kubectl.clone();
    cx.background_spawn(async move {
        kubectl.refresh();
    })
    .detach();
}
