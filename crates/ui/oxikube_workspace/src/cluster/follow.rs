//! [`follow_session`]: re-read one cluster's session when its badge or name changes.

use futures::{FutureExt as _, StreamExt as _};
use gpui::{Context, Task};
use oxikube_app::{ClusterSession, ClusterSessionManager, SessionChange, SessionUpdate};
use oxikube_domain::ids::ClusterId;

/// Whether `change` can alter what a badge or a label shows.
fn touches_badge(change: &SessionChange) -> bool {
    matches!(
        change,
        SessionChange::Opened
            | SessionChange::Closed
            | SessionChange::ReadOnlyChanged(_)
            | SessionChange::ColourChanged(_)
            | SessionChange::DisplayNameChanged(_)
    )
}

/// Follows the session of the cluster `cluster(&view)` names, for a view `T`.
///
/// Whenever that session's read-only flag, colour, display name, open state change,
/// `on_change` runs with a fresh snapshot (`None` when the session is closed). Updates for
/// other clusters, and connection-state noise, never wake the view. The updates that are
/// already waiting are drained together, so a burst (a preset writes colour and read-only at
/// once) calls `on_change` once, and the view calls `cx.notify()` once.
///
/// Keep the returned task in the view: dropping it stops following. It is never cleared from
/// inside itself.
pub fn follow_session<T: 'static>(
    manager: &ClusterSessionManager,
    cx: &mut Context<T>,
    cluster: impl Fn(&T) -> Option<ClusterId> + 'static,
    on_change: impl Fn(&mut T, Option<ClusterSession>, &mut Context<T>) + 'static,
) -> Task<()> {
    let manager = manager.clone();
    let mut updates = manager.subscribe();
    cx.spawn(async move |this, cx| {
        while let Some(first) = updates.next().await {
            let mut batch = vec![first];
            while let Some(Some(next)) = updates.next().now_or_never() {
                batch.push(next);
            }
            let alive = this.update(cx, |this, cx| {
                let Some(cluster) = cluster(this) else {
                    return;
                };
                let relevant = batch.iter().any(|item| match item {
                    // The subscriber missed updates: re-read to be safe.
                    Err(_) => true,
                    Ok(SessionUpdate { cluster: c, change }) => {
                        *c == cluster && touches_badge(change)
                    }
                });
                if relevant {
                    on_change(this, manager.get(&cluster), cx);
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
}
