//! [`LayoutPersistence`]: keeps a window's saved layout and its [`Workspace`] in step.
//!
//! - **Restore** on start: the saved layout is read through the [`LayoutStore`] (async, off the UI
//!   thread), then applied to the workspace with
//!   [`Workspace::restore_layout`](crate::Workspace::restore_layout). Until it finishes the
//!   controller is [`RestoreStatus::Restoring`] (the window shows its placeholder, E05-S13) and
//!   writes nothing, so an empty startup layout can never overwrite the saved one.
//! - **Save** on change: every workspace layout change and every window move or resize marks the
//!   layout dirty and (re)starts a debounce timer ([`SAVE_DEBOUNCE`]); when it fires the layout
//!   is captured on the UI thread (cheap: a tree walk) and written asynchronously. An unchanged
//!   layout is not written again.
//! - **Flush on quit**: an app-quit hook writes whatever is pending and waits for it.
//!
//! The controller is owned by the workspace it persists ([`Workspace::attach`]): hold nothing, and
//! it lives as long as the window's workspace does.
//!
//! Tasks: the debounce timer, the in-flight write and the restore each live in a field of this
//! entity and are never cleared from inside themselves (replacing the debounce timer from
//! `note_change` is what cancels the previous one).

use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, Subscription, Task,
    WeakEntity, Window,
};
use serde_json::Value;

use super::{LayoutStore, LoadOutcome, RestoreReport, SerializedWindow, SerializedWorkspace};
use crate::workspace::{Workspace, WorkspaceEvent};

/// How long a layout change waits for further changes before it is written.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// Where the startup restore stands.
#[derive(Clone, Debug, PartialEq)]
pub enum RestoreStatus {
    /// The saved layout is being read. Nothing is written yet.
    Restoring,
    /// A saved layout was applied.
    Restored(RestoreReport),
    /// Nothing was saved (first launch, or the state was reset).
    NothingSaved,
    /// A layout was stored but cannot be used (newer build, damaged); the default layout stays.
    Discarded(String),
    /// The state store failed; the default layout stays and saving resumes.
    Failed(String),
}

/// What the controller reports.
#[derive(Clone, Debug, PartialEq)]
pub enum PersistenceEvent {
    /// The startup restore finished with this (never [`RestoreStatus::Restoring`]) status.
    RestoreFinished(RestoreStatus),
}

/// See the [module docs](self).
pub struct LayoutPersistence {
    workspace: WeakEntity<Workspace>,
    store: LayoutStore,
    window: AnyWindowHandle,
    status: RestoreStatus,
    /// A change since the last write that the timer has not yet turned into one.
    dirty: bool,
    last_written: Option<Value>,
    debounce_task: Option<Task<()>>,
    write_task: Option<Task<()>>,
    restore_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PersistenceEvent> for LayoutPersistence {}

impl LayoutPersistence {
    /// Starts persisting `workspace`'s layout through `store` and begins the restore. Call it after
    /// the side panels were added and the item builders registered (restore needs both); the
    /// workspace keeps working while the layout loads.
    pub fn start(
        workspace: &Entity<Workspace>,
        store: LayoutStore,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let this = cx.new(|cx| {
            let subscriptions = vec![
                cx.subscribe_in(
                    workspace,
                    window,
                    |this: &mut Self, _, _: &WorkspaceEvent, window, cx| {
                        this.note_change(window, cx)
                    },
                ),
                cx.observe_window_bounds(window, |this: &mut Self, window, cx| {
                    this.note_change(window, cx)
                }),
                cx.on_app_quit(|this: &mut Self, cx| this.on_quit(cx)),
            ];
            Self {
                workspace: workspace.downgrade(),
                store,
                window: window.window_handle(),
                status: RestoreStatus::Restoring,
                dirty: false,
                last_written: None,
                debounce_task: None,
                write_task: None,
                restore_task: None,
                _subscriptions: subscriptions,
            }
        });
        this.update(cx, |this, cx| this.begin_restore(window, cx));
        // The workspace owns the controller: it lives exactly as long as the window's layout does,
        // and nothing else has to remember to hold it.
        workspace.update(cx, |ws, _| ws.attach(this.clone()));
        this
    }

    /// Where the startup restore stands.
    pub fn status(&self) -> &RestoreStatus {
        &self.status
    }

    /// Whether the saved layout is still being read (show the placeholder).
    pub fn is_restoring(&self) -> bool {
        self.status == RestoreStatus::Restoring
    }

    fn begin_restore(&mut self, window: &Window, cx: &mut Context<Self>) {
        let store = self.store.clone();
        self.restore_task = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = store.load().await;
            this.update_in(cx, |this, window, cx| {
                this.finish_restore(loaded, window, cx)
            })
            .ok();
        }));
    }

    fn finish_restore(
        &mut self,
        loaded: oxikube_domain::OxiResult<LoadOutcome>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status = match loaded {
            Ok(LoadOutcome::Loaded(layout)) => {
                match self
                    .workspace
                    .update(cx, |ws, cx| ws.restore_layout(&layout, window, cx))
                {
                    Ok(report) => RestoreStatus::Restored(report),
                    Err(_) => RestoreStatus::Failed("the workspace is gone".into()),
                }
            }
            Ok(LoadOutcome::Missing) => RestoreStatus::NothingSaved,
            Ok(LoadOutcome::Discarded(error)) => RestoreStatus::Discarded(error.to_string()),
            Err(error) => {
                tracing::warn!(%error, "layout restore failed: starting with the default layout");
                RestoreStatus::Failed(error.to_string())
            }
        };
        if matches!(&self.status, RestoreStatus::Restored(report) if !report.centre_kept) {
            // What was just restored is what is stored: the layout events the restore itself
            // causes (dock resizes, window bounds) must not write it back, even when every saved
            // item was skipped. (Skipped items thus stay in the store until the user changes
            // something, so a feature crate that is briefly missing loses nothing. When the
            // user's own items were kept, the layout differs from the stored one on purpose and
            // is written.)
            self.last_written = self.capture(window, cx).map(|layout| layout.to_json());
        }
        cx.emit(PersistenceEvent::RestoreFinished(self.status.clone()));
        if self.dirty {
            self.note_change(window, cx);
        }
        cx.notify();
    }

    /// The layout as it is now, window included; `None` once the workspace is gone.
    fn capture(&self, window: &Window, cx: &App) -> Option<SerializedWorkspace> {
        let mut layout = self.workspace.upgrade()?.read(cx).serialize_layout(cx);
        layout.window = Some(SerializedWindow::from_window_bounds(window.window_bounds()));
        Some(layout)
    }

    /// Something changed: write soon, once changes stop for the debounce period.
    fn note_change(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.dirty = true;
        if self.is_restoring() {
            // Written once the restore is done (`finish_restore`).
            return;
        }
        // Replacing the previous timer cancels it. The timer only dispatches; the write is a
        // separate task, so nothing here is dropped from inside itself.
        self.debounce_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            this.update_in(cx, |this, window, cx| {
                // Only a new write replaces the in-flight one (which a newer layout supersedes).
                if let Some(write) = this
                    .capture(window, cx)
                    .and_then(|snapshot| this.spawn_write(snapshot, cx))
                {
                    this.write_task = Some(write);
                }
            })
            .ok();
        }));
    }

    /// Writes any pending change now, without waiting for the debounce, and returns the write.
    /// Hold the task until it resolves: dropping it cancels the write. Resolves at once when
    /// nothing is pending.
    pub fn flush(&mut self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        self.debounce_task = None;
        if self.is_restoring() {
            return Task::ready(());
        }
        self.capture(window, cx)
            .and_then(|snapshot| self.spawn_write(snapshot, cx))
            .unwrap_or_else(|| Task::ready(()))
    }

    /// The app-quit hook: writes the layout as it is now, and the quit waits for it. The future
    /// awaits the store directly (no task of ours), so it completes inside the quit's bounded wait.
    fn on_quit(&mut self, cx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + use<> {
        self.debounce_task = None;
        let snapshot = if self.is_restoring() {
            None
        } else {
            let this = &*self;
            // The window may already be going away; then the last debounced write is what stays.
            this.window
                .update(cx, |_, window, cx| this.capture(window, cx))
                .ok()
                .flatten()
        };
        // A write that is still in flight might not have reached the store: write again rather
        // than trust it. Only a layout that matches the last one *and* has no write outstanding is
        // skipped.
        let unchanged = snapshot
            .as_ref()
            .is_some_and(|s| self.last_written.as_ref() == Some(&s.to_json()))
            && self.write_task.is_none();
        let store = self.store.clone();
        async move {
            let Some(snapshot) = snapshot.filter(|_| !unchanged) else {
                return;
            };
            if let Err(error) = store.save(&snapshot).await {
                tracing::warn!(%error, "saving the layout on quit failed");
            }
        }
    }

    /// Starts writing `snapshot` unless it equals what was last written (or is being written).
    fn spawn_write(
        &mut self,
        snapshot: SerializedWorkspace,
        cx: &mut Context<Self>,
    ) -> Option<Task<()>> {
        let json = snapshot.to_json();
        if self.last_written.as_ref() == Some(&json) {
            self.dirty = false;
            return None;
        }
        self.last_written = Some(json);
        self.dirty = false;
        let store = self.store.clone();
        Some(cx.spawn(async move |this, cx| {
            if let Err(error) = store.save(&snapshot).await {
                tracing::warn!(%error, "saving the layout failed");
                // Not written: let the next change (or flush) try again.
                this.update(cx, |this, _| this.last_written = None).ok();
            }
        }))
    }
}
