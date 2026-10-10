//! A jump to a context that is not connected: `:pods @staging` connects it, and opens the list
//! once its session is up.
//!
//! The wait is a task the host keeps (a newer jump replaces it, which cancels it). It listens to
//! the session updates, so a connect that finishes at once, a slow one and one that fails all end
//! it: success sends the rest of the plan, a failure or a timeout says so in a toast (the
//! cluster's own tab shows why it failed).

use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::future::{Either, select};
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_app::search::jump::AfterConnect;
use oxikube_app::session::{SessionChange, SessionLagged, SessionUpdate};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::SessionPhase;
use oxikube_workspace::{CommandDispatcher, Toast, Workspace};

use super::host::show_toast;

/// How long a jump waits for a connection before it gives up. A connect that is still going
/// after that shows its own progress in the cluster's tab.
pub const CONNECT_WAIT: Duration = Duration::from_secs(60);

/// Where a connection stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Connected,
    /// It needs the user (credentials) or failed and will not retry by itself.
    Failed,
    /// Not open yet, connecting, or failed and about to retry.
    Waiting,
}

fn status(sessions: &ClusterSessionManager, cluster: &ClusterId) -> Status {
    let Some(session) = sessions.get(cluster) else {
        return Status::Waiting;
    };
    match session.phase() {
        phase if phase.is_connected() => Status::Connected,
        SessionPhase::AuthRequired => Status::Failed,
        SessionPhase::Error if session.auto_reconnect().is_none() => Status::Failed,
        _ => Status::Waiting,
    }
}

/// Whether an update says the connection state of `cluster` moved (or that updates were missed,
/// so the state may have).
fn moved_state(update: &Result<SessionUpdate, SessionLagged>, cluster: &ClusterId) -> bool {
    match update {
        Ok(update) => {
            update.cluster == *cluster
                && matches!(update.change, SessionChange::StateChanged { .. })
        }
        Err(SessionLagged { .. }) => true,
    }
}

/// Sends `after.commands` through `dispatcher` once `after.cluster` is connected; a toast in
/// `workspace` says so when it did not connect in time.
pub(super) fn when_connected(
    after: AfterConnect,
    dispatcher: Rc<dyn CommandDispatcher>,
    sessions: ClusterSessionManager,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    window.spawn(cx, async move |cx| {
        // Subscribe before looking, so a connection that completes in between is not missed.
        let mut updates = sessions.subscribe();
        let mut deadline = std::pin::pin!(cx.background_executor().timer(CONNECT_WAIT));
        // A session left in a failed state by an earlier attempt is still there when the connect
        // is only queued (the command bus defers it): that failure is old news, and counts only
        // once the session has moved on and failed again.
        let mut stale_failure = status(&sessions, &after.cluster) == Status::Failed;
        let arrived = loop {
            match status(&sessions, &after.cluster) {
                Status::Connected => break true,
                Status::Failed if !stale_failure => break false,
                Status::Failed | Status::Waiting => {}
            }
            let next = std::pin::pin!(updates.next());
            match select(next, deadline.as_mut()).await {
                Either::Left((Some(update), _)) => {
                    stale_failure &= !moved_state(&update, &after.cluster);
                }
                Either::Left((None, _)) | Either::Right(_) => {
                    break status(&sessions, &after.cluster) == Status::Connected;
                }
            }
        };
        cx.update(|_, cx| {
            if arrived {
                for command in &after.commands {
                    dispatcher.dispatch(command.clone(), cx);
                }
            } else {
                let toast =
                    Toast::warning("The cluster did not connect, so the jump was not made.");
                show_toast(&workspace, toast, cx);
            }
        })
        .ok();
    })
}
