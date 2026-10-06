//! The session update stream: [`SessionUpdate`], [`SessionChange`], [`SessionUpdates`].
//!
//! Every change the manager makes to a session is broadcast as a [`SessionUpdate`]
//! carrying the session's [`ClusterId`]. Updates of one session arrive in the order they
//! happened (they are sent under that session's lock). The broadcast is bounded: a
//! subscriber that falls behind gets one [`SessionLagged`] and should re-read
//! [`sessions`](super::ClusterSessionManager::sessions) instead of replaying.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::{BoxStream, Stream, StreamExt};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, NamespaceSelection, SessionPhase};
use oxikube_domain::{Capabilities, ClusterColour};
use tokio::sync::broadcast;

/// One change to one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUpdate {
    /// The session that changed.
    pub cluster: ClusterId,
    /// What changed.
    pub change: SessionChange,
}

/// What changed about a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionChange {
    /// The session was opened (it starts `Disconnected`).
    Opened,
    /// The session was closed and forgotten.
    Closed,
    /// The connection state moved. Self-transitions (`Ready` + `Healthy`) are not sent.
    StateChanged {
        /// The phase before.
        from: SessionPhase,
        /// The new state, with its reason for `AuthRequired` and `Error`.
        state: ClusterSessionState,
    },
    /// The capabilities changed: probed on connect, cleared on disconnect.
    CapabilitiesChanged(Capabilities),
    /// The namespace selection changed.
    NamespaceChanged(NamespaceSelection),
    /// The read-only flag changed.
    ReadOnlyChanged(bool),
    /// The colour changed.
    ColourChanged(Option<ClusterColour>),
    /// The display name changed (`None`: the context name is shown).
    DisplayNameChanged(Option<String>),
}

/// The subscriber fell behind and missed `missed` updates; re-read the sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLagged {
    /// How many updates were dropped for this subscriber.
    pub missed: u64,
}

/// A subscription to every session's updates. Ends when the manager is dropped.
pub struct SessionUpdates {
    inner: BoxStream<'static, Result<SessionUpdate, SessionLagged>>,
}

impl SessionUpdates {
    fn new(rx: broadcast::Receiver<SessionUpdate>) -> Self {
        let inner = futures::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok(update) => Some((Ok(update), rx)),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    Some((Err(SessionLagged { missed }), rx))
                }
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed();
        Self { inner }
    }
}

impl std::fmt::Debug for SessionUpdates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionUpdates").finish_non_exhaustive()
    }
}

impl Stream for SessionUpdates {
    type Item = Result<SessionUpdate, SessionLagged>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

/// The sending half, owned by the manager.
pub(super) struct UpdateSender(broadcast::Sender<SessionUpdate>);

impl UpdateSender {
    pub(super) fn new(capacity: usize) -> Self {
        Self(broadcast::channel(capacity.max(1)).0)
    }

    pub(super) fn subscribe(&self) -> SessionUpdates {
        SessionUpdates::new(self.0.subscribe())
    }

    /// Sends one update; having no subscriber is fine.
    pub(super) fn send(&self, cluster: &ClusterId, change: SessionChange) {
        let _ = self.0.send(SessionUpdate {
            cluster: cluster.clone(),
            change,
        });
    }
}
