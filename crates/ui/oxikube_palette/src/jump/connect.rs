//! A jump to a context that is not connected: `:pods @staging` connects it, and opens the list
//! once its session is up.
//!
//! The wait is a task the host keeps (a newer jump replaces it, which cancels it). It listens to
//! the session updates, so a connect that finishes at once, a slow one and one that fails all end
//! it: success sends the rest of the plan, a failure or a timeout says so in a toast (the
//! cluster's own tab shows why it failed).
//!
//! A line whose resource could not be resolved before the connection (`:certs @staging`, a CRD
//! alias that only the cluster's discovery knows) is planned again once the session is up. The
//! alias table follows the connection a moment later, so an alias that is still unknown is tried
//! again for [`ALIAS_WAIT`] before the error is shown.

use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::future::{Either, select};
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_app::search::jump::{AfterConnect, JumpPlan, ParseError, ParseErrorKind};
use oxikube_app::session::{SessionChange, SessionLagged, SessionUpdate};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::SessionPhase;
use oxikube_workspace::{CommandDispatcher, Toast, Workspace};

use super::host::show_toast;

/// How long a jump waits for a connection before it gives up. A connect that is still going
/// after that shows its own progress in the cluster's tab.
pub const CONNECT_WAIT: Duration = Duration::from_secs(60);

/// How long a line planned again after the connection waits for the cluster's aliases to arrive.
pub const ALIAS_WAIT: Duration = Duration::from_secs(5);

/// How often it tries.
const ALIAS_POLL: Duration = Duration::from_millis(100);

/// Plans a line against the app as it is now.
pub(super) type Replan = Rc<dyn Fn(&str, &App) -> Result<JumpPlan, ParseError>>;

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
    replan: Replan,
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
        if !arrived {
            cx.update(|_, cx| {
                let toast =
                    Toast::warning("The cluster did not connect, so the jump was not made.");
                show_toast(&workspace, toast, cx);
            })
            .ok();
            return;
        }
        let commands = match &after.replan {
            Some(line) => planned_again(line, &replan, cx).await,
            None => Ok(after.commands.clone()),
        };
        cx.update(|_, cx| match commands {
            Ok(commands) => {
                for command in commands {
                    dispatcher.dispatch(command, cx);
                }
            }
            Err((line, error)) => {
                let toast = Toast::warning(format!("`{line}`: {error}"));
                show_toast(&workspace, toast, cx);
            }
        })
        .ok();
    })
}

/// Plans `line` against the connected cluster; an alias it does not know yet is tried again for
/// [`ALIAS_WAIT`], as the alias table follows the connection in the background.
async fn planned_again(
    line: &str,
    replan: &Replan,
    cx: &mut gpui::AsyncWindowContext,
) -> Result<Vec<oxikube_domain::command::Command>, (String, ParseError)> {
    let started = cx.background_executor().now();
    loop {
        let planned = cx.update(|_, cx| replan(line, cx));
        let error = match planned {
            Ok(Ok(plan)) => return Ok(plan.commands),
            Ok(Err(error)) => error,
            // The window is gone.
            Err(_) => return Ok(Vec::new()),
        };
        let aliases_may_follow = matches!(
            error.kind,
            ParseErrorKind::UnknownAlias | ParseErrorKind::NotServed
        );
        let waited = cx.background_executor().now().duration_since(started);
        if !aliases_may_follow || waited >= ALIAS_WAIT {
            return Err((line.to_owned(), error));
        }
        cx.background_executor().timer(ALIAS_POLL).await;
    }
}
