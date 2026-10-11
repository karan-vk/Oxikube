//! Following the node and the pods on it.

use gpui::Context;
use oxikube_app::actions::ActionContext;
use oxikube_app::nodes::list_node_pods;
use oxikube_domain::command::{self, CommandId};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_runtime::spawn_kube;

use super::data::{Followed, NodeData, PodsOnNode};
use super::{Fetch, NodePanel};

/// Waiting ticks (a quarter of a second each) before the node's pods are listed from the API
/// server: the Pod feed gets a second and a half to answer.
const LIVE_PATIENCE: u32 = 6;

impl NodePanel {
    /// Brings the panel up to date with the detail's node, the session and the pods. Cheap when
    /// nothing moved (the tick calls it once a second).
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let mut changed = false;
        let store = {
            let view = detail.read(cx);
            let current = view.resource();
            let stale = match (&self.node, current) {
                (Some(node), Some(res)) => {
                    node.version.is_none() || node.version != res.meta.resource_version
                }
                (None, Some(_)) => true,
                _ => false,
            };
            if stale && let Some(res) = current {
                self.node = NodeData::of(res);
                changed = true;
            }
            view.store().cloned()
        };
        changed |= self.follow_store(store);
        changed |= self.follow_pods(cx);
        changed |= self.follow_session(&detail, cx);
        if changed {
            cx.notify();
        }
    }

    /// Takes the lease on the pods-by-node index of the session's store (a reconnect is a new
    /// store: the lease moves with it and what was fetched is dropped).
    fn follow_store(&mut self, store: Option<oxikube_app::store::ResourceStore>) -> bool {
        let same = match (&self.store, &store) {
            (Some(a), Some(b)) => a.is_same(b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return false;
        }
        self.lease = store.as_ref().map(|s| s.lease_node_loads());
        self.store = store;
        self.pods = None;
        self.followed = None;
        self.waited = 0;
        self.fetch = Fetch::Idle;
        true
    }

    /// Rebuilds the pod rows when the live index says they changed; starts the fallback list
    /// when it has been silent too long.
    fn follow_pods(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(name) = self.node.as_ref().map(|n| n.summary.name.clone()) else {
            return false;
        };
        let live = self.lease.as_ref().and_then(|lease| lease.load(&name));
        if let Some((demand, pods)) = live {
            self.waited = 0;
            let unchanged = self
                .followed
                .as_ref()
                .is_some_and(|f| f.is_current(&demand, &pods));
            if unchanged && self.pods.as_ref().is_some_and(|p| p.live) {
                return false;
            }
            self.followed = Some(Followed::of(demand, &pods));
            self.pods = Some(PodsOnNode::from_objects(demand, &pods));
            self.fetch = Fetch::Idle;
            return true;
        }
        self.waited = self.waited.saturating_add(1);
        if self.waited >= LIVE_PATIENCE && matches!(self.fetch, Fetch::Idle) {
            self.fetch_pods(name.to_string(), cx);
        }
        false
    }

    /// Lists the pods on the node from the API server, once.
    fn fetch_pods(&mut self, node: String, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (cluster, sessions) = {
            let view = detail.read(cx);
            (view.target().cluster.clone(), view.deps().sessions.clone())
        };
        let Some(reader) = sessions.get(&cluster).and_then(|s| s.resources()) else {
            return;
        };
        let read = spawn_kube(cx, async move { list_node_pods(&*reader, &node).await });
        self.fetch = Fetch::Loading(cx.spawn(async move |this, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |panel, cx| panel.fetched(result, cx)).ok();
        }));
    }

    fn fetched(&mut self, result: OxiResult<oxikube_app::nodes::NodePods>, cx: &mut Context<Self>) {
        match result {
            // The live index may have answered meanwhile: it wins.
            Ok(found) if !self.pods.as_ref().is_some_and(|p| p.live) => {
                self.pods = Some(PodsOnNode::from_list(&found));
                self.fetch = Fetch::Done;
            }
            Ok(_) => self.fetch = Fetch::Done,
            Err(error) => {
                if error.kind() != ErrorKind::Forbidden {
                    tracing::warn!(%error, "listing the pods on a node failed");
                }
                self.fetch = Fetch::Failed(error.message().to_owned());
            }
        }
        cx.notify();
    }

    /// Whether cordon and uncordon are blocked (read-only) right now.
    fn follow_session(
        &mut self,
        detail: &gpui::Entity<crate::detail::DetailView>,
        cx: &mut Context<Self>,
    ) -> bool {
        let (cluster, sessions) = {
            let view = detail.read(cx);
            (view.target().cluster.clone(), view.deps().sessions.clone())
        };
        let blocked = sessions.get(&cluster).and_then(|session| {
            let meta = command::lookup(CommandId::NODE_CORDON)?;
            if !session.capabilities().contains(meta.needs) {
                return Some("Your permissions do not allow changing nodes".to_owned());
            }
            ActionContext::of(&session)
                .state_of(meta)
                .reason()
                .map(|reason| reason.to_string())
        });
        let changed = blocked != self.blocked;
        self.blocked = blocked;
        changed
    }
}
