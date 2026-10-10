//! [`SessionEcho`]: the session updates an immediate command made, handed to the views in the
//! update that ran it (E05-P600).
//!
//! Views follow their session through the manager's update stream, which a GPUI task drains when
//! the executor next polls it: after the current update and, for an input dispatched just before a
//! frame, after that frame. An immediate command (`namespace::Select` run by the UI) changes the
//! session on the UI thread, so its update is already in the stream when the command returns.
//! The caller brackets the command with [`SessionEcho::begin`] and [`SessionEcho::finish`]; every
//! update sent in between goes to the views that [`observe_session_echo`], before the update
//! ends, so the frame drawn right after the input shows it.
//!
//! The views' own streams deliver the same updates again later. A view applies an echoed update
//! the way it applies a streamed one, and both paths are idempotent (a scope already applied is
//! not applied again), so the second delivery costs a comparison.

use std::rc::Rc;

use futures::{FutureExt as _, StreamExt as _};
use gpui::{App, Context, Global, Subscription};
use oxikube_app::session::SessionLagged;
use oxikube_app::{ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates};
use oxikube_domain::ids::ClusterId;

/// One echoed item: an update, or a note that the recording missed some (re-read the sessions).
pub type EchoItem = Result<SessionUpdate, SessionLagged>;

/// Whether `items` hold a namespace change of `cluster`'s session, or a note that some were
/// missed (then the sessions must be re-read). What the views that follow the namespace selection
/// ask of an echo.
pub fn namespace_changed(items: &[EchoItem], cluster: &ClusterId) -> bool {
    items.iter().any(|item| match item {
        Ok(update) => {
            update.cluster == *cluster
                && matches!(update.change, SessionChange::NamespaceChanged(_))
        }
        Err(_) => true,
    })
}

/// Records the session updates sent while a command runs on the UI thread. See the
/// module docs.
#[must_use = "`finish` hands the updates to the views"]
pub struct SessionEcho {
    updates: SessionUpdates,
}

impl SessionEcho {
    /// Starts recording what `sessions` sends from now on.
    pub fn begin(sessions: &ClusterSessionManager) -> Self {
        Self {
            updates: sessions.subscribe(),
        }
    }

    /// Hands every update sent since [`begin`](Self::begin) to the views observing the echo.
    /// Their callbacks run when the current update's effects are flushed, before it returns.
    /// Nothing happens when nothing was sent.
    pub fn finish(mut self, cx: &mut App) {
        let mut items = Vec::new();
        while let Some(Some(item)) = self.updates.next().now_or_never() {
            items.push(item);
        }
        if !items.is_empty() {
            cx.set_global(Echoed(items.into()));
        }
    }
}

/// The last echo, read by the observers when it is set.
struct Echoed(Rc<[EchoItem]>);

impl Global for Echoed {}

/// Calls `on_echo` with the updates of every [`SessionEcho`] finished from now on, in the update
/// that finished it. Keep the returned subscription in the view.
pub fn observe_session_echo<T: 'static>(
    cx: &mut Context<T>,
    on_echo: impl Fn(&mut T, &[EchoItem], &mut Context<T>) + 'static,
) -> Subscription {
    cx.observe_global::<Echoed>(move |this, cx| {
        let items = cx.global::<Echoed>().0.clone();
        on_echo(this, &items, cx);
    })
}
