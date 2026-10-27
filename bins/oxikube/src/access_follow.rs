//! The access registry's session follower in the running app (E17 audit).
//!
//! [`start`] runs on each window mount and does its work once per app: the
//! [`AccessRegistry`](oxikube_app::access::AccessRegistry) the namespace service owns drops the
//! access service (and the kube client pool, rules cache and discovery its port holds) of every
//! cluster that is closed or disconnected. The follower only waits, so it runs on the foreground
//! executor.

use gpui::{App, Global, Task};
use oxikube_app::session::namespaces::NamespaceService;

/// What [`start`] keeps running for the life of the app.
struct AccessFollow {
    _sessions: Task<()>,
}

impl Global for AccessFollow {}

/// Starts the follower (idempotent: a second window reuses what the first started).
pub fn start(namespaces: &NamespaceService, cx: &mut App) {
    if cx.has_global::<AccessFollow>() {
        return;
    }
    let follow = namespaces.access().follow(namespaces.manager());
    let sessions = cx.spawn(async move |_| follow.await);
    cx.set_global(AccessFollow {
        _sessions: sessions,
    });
}
