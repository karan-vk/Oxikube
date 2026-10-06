//! Session restore on the window's side (E06-S11): placeholder tabs for the clusters of the last
//! session, connecting them lazily, and the notice for the ones that vanished.
//!
//! The app layer decides *what* to restore ([`SessionRestorer`]); this file puts it on screen
//! without ever blocking the UI thread or the first frame:
//!
//! 1. [`ClusterTabs::restore_session`] returns at once. If `session.restore` is off (the default)
//!    it does nothing, ever.
//! 2. Its task waits for the first frame to be drawn and for the layout restore (E05-S05) to
//!    finish, then reads the saved session on `spawn_kube`.
//! 3. [`hold_placeholders`](ClusterTabs::hold_placeholders) opens a tab for every restored
//!    cluster, in the saved order, the saved one displayed. A tab whose cluster has not connected
//!    is a placeholder (its session is `Disconnected`, which would normally close the tab).
//! 4. The displayed cluster connects, on `spawn_kube`; with `session.restore_connect = "all"`
//!    the others do too, two at a time. By default a placeholder connects when its tab is first
//!    shown: that sends `cluster::Connect` through the dispatcher, like the catalog does.
//! 5. A cluster that fails shows `Error` (or `AuthRequired`) in its own tab and affects no other.
//!
//! A placeholder stays until its cluster connects (then it is an ordinary tab) or the user closes
//! it (no disconnect to send: it is not connected).

use std::{cell::RefCell, rc::Rc};

use futures::channel::oneshot;
use gpui::{Context, Window};
use oxikube_app::session::restore::{RestoreConnect, RestorePlan, SessionRestorer};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::SessionPhase;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_runtime::spawn_kube;
use oxikube_settings::Settings as _;

use super::ClusterTabs;
use crate::cluster_tab::restore_settings::SessionRestoreSettings;
use crate::persistence::{LayoutPersistence, PersistenceEvent, RestoreStatus};
use crate::toast::Toast;

impl ClusterTabs {
    /// Reopens the last session, if `session.restore` is on: see the [file docs](self). Call it
    /// once the window exists, after [`ClusterTabs::start`] and after the layout persistence
    /// started (`layout`; `None` when there is none to wait for). Returns at once; everything
    /// runs after the first frame.
    ///
    /// Nothing here changes a cluster, so it is no `MutationGuard` concern, and it is not a user
    /// action: the setting is the opt-in, so there is no command for it.
    pub fn restore_session(
        &mut self,
        restorer: SessionRestorer,
        layout: Option<gpui::Entity<LayoutPersistence>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !SessionRestoreSettings::try_get(cx).is_some_and(|settings| settings.restore) {
            return;
        }
        // Held in a field and only replaced from outside the task, never cleared by it.
        self.restore_task = Some(cx.spawn_in(window, async move |this, cx| {
            // Nothing below runs before the first frame is on screen and the layout is back.
            let frame = this.update_in(cx, |_, window, _| next_frame(window));
            let Ok(frame) = frame else { return };
            frame.await.ok();
            let waiting = this.update(cx, |_, cx| layout_restored(layout.as_ref(), cx));
            if let Ok(Some((restored, _subscription))) = waiting {
                restored.await.ok();
            }

            let prepare = restorer.clone();
            let plan = flatten(spawn_kube(cx, async move { prepare.prepare().await }).await);
            let plan = match plan {
                Ok(plan) => plan,
                Err(error) => {
                    // Never blocks the app: it starts with what it has.
                    tracing::warn!(%error, "the last session could not be restored");
                    return;
                }
            };
            let connect = this.update_in(cx, |this, window, cx| {
                this.hold_placeholders(&plan, window, cx);
                this.tell_dropped(&plan, cx);
                SessionRestoreSettings::try_get(cx)
                    .map_or(RestoreConnect::Active, |settings| settings.connect.into())
            });
            let Ok(connect) = connect else { return };
            let report = spawn_kube(cx, async move { restorer.connect(&plan, connect).await })
                .await
                .ok();
            for outcome in report.iter().flat_map(|report| report.failures()) {
                tracing::info!(
                    cluster = %outcome.cluster,
                    state = ?outcome.state.phase(),
                    "a restored cluster did not connect"
                );
            }
        }));
    }

    /// Opens a placeholder tab for every cluster of `plan` that has an open, disconnected session
    /// and no tab yet, in the plan's order, then shows the plan's displayed cluster (or what was
    /// displayed before, when the catalog was). Nothing connects here.
    pub fn hold_placeholders(
        &mut self,
        plan: &RestorePlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let before = workspace
            .read(cx)
            .active_item(cx)
            .map(|item| item.item_id());
        // The user may have connected and switched to a cluster while the restore was loading:
        // what they are looking at stays.
        let user_is_on_a_cluster = self.active.is_some();
        self.restore_active = plan.active.clone();
        for cluster in plan.ids() {
            if self.tabs.contains_key(cluster) {
                continue;
            }
            let Some(session) = self.deps.sessions.get(cluster) else {
                continue;
            };
            if session.phase() == SessionPhase::Disconnected {
                self.pending.insert(cluster.clone());
            }
            self.follow(&session, window, cx);
        }
        let saved_active = plan
            .active
            .as_ref()
            .filter(|active| !user_is_on_a_cluster && self.tabs.contains_key(*active));
        match saved_active {
            Some(active) => {
                let active = active.clone();
                self.activate(&active, window, cx);
            }
            // The catalog (or whatever was shown) stays shown.
            None => {
                if let Some(before) = before {
                    workspace.update(cx, |ws, cx| ws.activate_item(before, false, window, cx));
                }
            }
        }
    }

    /// Whether `cluster`'s tab is a restored placeholder: shown, not connected yet.
    pub fn is_placeholder(&self, cluster: &ClusterId) -> bool {
        self.pending.contains(cluster)
    }

    /// A tab became the displayed one. A placeholder that is shown for the first time connects
    /// its cluster, unless the restore itself is connecting it.
    pub(super) fn on_displayed(&mut self, cluster: &ClusterId, cx: &mut Context<Self>) {
        let shown = self
            .tabs
            .get(cluster)
            .is_some_and(|entry| entry.tab.read(cx).is_active());
        // A tab that was displayed only on the way to the saved one is not "shown".
        if !shown
            || !self.pending.contains(cluster)
            || self.restore_active.as_ref() == Some(cluster)
        {
            return;
        }
        self.deps.dispatcher.dispatch(
            Command::ClusterConnect {
                cluster: cluster.clone(),
            },
            cx,
        );
    }

    /// Tells the user which saved clusters no kubeconfig defines any more.
    fn tell_dropped(&self, plan: &RestorePlan, cx: &mut Context<Self>) {
        if plan.dropped.is_empty() {
            return;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let names: Vec<String> = plan.dropped.iter().map(|d| d.label()).collect();
        let (title, message) = match names.as_slice() {
            [one] => (
                "A saved cluster was removed".to_owned(),
                format!("{one} is not in any kubeconfig any more, so its tab was not reopened."),
            ),
            many => (
                format!("{} saved clusters were removed", many.len()),
                format!(
                    "{} are not in any kubeconfig any more, so their tabs were not reopened.",
                    many.join(", ")
                ),
            ),
        };
        let toast = Toast::warning(message)
            .title(title)
            .key("session-restore/dropped");
        workspace.update(cx, |ws, cx| {
            ws.show_toast(toast, cx);
        });
    }
}

/// Resolves after the next frame is drawn.
fn next_frame(window: &mut Window) -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    window.on_next_frame(move |_, _| {
        tx.send(()).ok();
    });
    rx
}

/// A receiver that resolves when the layout restore finishes, or `None` when it already has (or
/// there is none). The subscription must be held until then.
fn layout_restored(
    layout: Option<&gpui::Entity<LayoutPersistence>>,
    cx: &mut Context<ClusterTabs>,
) -> Option<(oneshot::Receiver<()>, gpui::Subscription)> {
    let layout = layout?;
    if *layout.read(cx).status() != RestoreStatus::Restoring {
        return None;
    }
    let (tx, rx) = oneshot::channel();
    let tx = Rc::new(RefCell::new(Some(tx)));
    let subscription = cx.subscribe(layout, move |_, _, _: &PersistenceEvent, _| {
        if let Some(tx) = tx.borrow_mut().take() {
            tx.send(()).ok();
        }
    });
    Some((rx, subscription))
}

/// A `spawn_kube` result with the bridge's own failure folded into the error.
fn flatten<T>(result: Result<OxiResult<T>, oxikube_runtime::KubeTaskError>) -> OxiResult<T> {
    result.map_err(OxiError::from).and_then(|inner| inner)
}
