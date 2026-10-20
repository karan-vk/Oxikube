//! The favourites over the fakes: an in-memory state store, the manager's fake backend and
//! port, a manual clock and the notification hooks' fake.

mod commands;
mod follow;
mod restore;
mod service;

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_testkit::{FakeStatePort, SourceCall};

use super::PortForwardFavourites;
use crate::portforward::tests::{NS, Rig, cluster};
use crate::store::Spawner;

/// The rig's manager with favourites over a fake state store.
pub(super) struct Favs {
    pub rig: Rig,
    pub state: Arc<FakeStatePort>,
    pub favourites: PortForwardFavourites,
}

impl Favs {
    pub fn new() -> Self {
        let rig = Rig::new();
        let state = Arc::new(FakeStatePort::new());
        let spawner: Arc<dyn Spawner> =
            Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
                tokio::spawn(task);
            });
        let favourites = PortForwardFavourites::new(
            state.clone(),
            rig.manager.clone(),
            spawner,
            rig.clock.clone(),
            Some(rig.notes.clone()),
        );
        Self {
            rig,
            state,
            favourites,
        }
    }

    /// The rig's forward-failed notifications as `(dedupe key, title-ish reason)`.
    pub fn failures(&self) -> Vec<(String, String)> {
        self.rig
            .notes
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                SourceCall::PortForward(c) if c.is_drop() => {
                    Some((c.dedupe_key(), c.reason.unwrap_or_default()))
                }
                _ => None,
            })
            .collect()
    }
}

pub(super) fn pod_in(cluster: ClusterId, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), NS, name)
}

/// A spec forwarding `name`'s port `remote` on a free local port, in the rig's cluster.
pub(super) fn spec(name: &str, remote: u16) -> ForwardSpec {
    ForwardSpec::new(pod_in(cluster(), name), ForwardPort::Number(remote))
}

/// Polls `done` (real time, the IO is real) for up to 5 s.
pub(super) async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never happened: {what}");
}
