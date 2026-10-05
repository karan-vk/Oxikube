//! The restart hook: follows the pod set of a forward and decides when the target is gone
//! and where to go next.

use std::net::SocketAddr;

use oxikube_domain::ForwardStatus;
use tokio::sync::watch;

use super::hub::StatusHub;
use super::plan::{Plan, PodInfo, Target};

/// What the session does after a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    Continue,
    /// The forward is over: a pod forward lost its pod.
    Stop,
}

/// Tracks the current target of one forward.
///
/// A service forward sticks to its pod while that pod can serve and only moves after
/// `TargetGone`; with no replacement it waits for the next snapshot (the watch pushes one on
/// every pod change), so there is no polling and no retry loop to spin.
pub(super) struct Monitor {
    plan: Plan,
    hub: StatusHub,
    target: watch::Sender<Option<Target>>,
    local_addr: SocketAddr,
    current: Option<Target>,
}

impl Monitor {
    /// A monitor already forwarding to `initial`; publishes `Listening`.
    pub(super) fn start(
        plan: Plan,
        hub: StatusHub,
        target: watch::Sender<Option<Target>>,
        local_addr: SocketAddr,
        initial: Target,
    ) -> Self {
        let mut monitor = Self {
            plan,
            hub,
            target,
            local_addr,
            current: None,
        };
        monitor.switch_to(initial);
        monitor
    }

    fn switch_to(&mut self, target: Target) {
        self.hub.publish(ForwardStatus::Listening {
            local_addr: self.local_addr,
            pod: target.pod.clone(),
        });
        self.target.send_replace(Some(target.clone()));
        self.current = Some(target);
    }

    /// Reacts to the current pods.
    pub(super) fn on_snapshot(&mut self, pods: &[PodInfo]) -> Flow {
        if let Some(current) = &self.current {
            let serving = pods
                .iter()
                .find(|pod| pod.name == current.pod)
                .is_some_and(|pod| self.plan.target_on(pod).is_some());
            if serving {
                return Flow::Continue;
            }
            let pod = current.pod.clone();
            self.current = None;
            self.target.send_replace(None);
            self.hub.publish(ForwardStatus::TargetGone { pod });
            if !self.plan.is_service() {
                return Flow::Stop;
            }
        }
        if let Some(target) = self.plan.pick(pods) {
            self.switch_to(target);
        }
        Flow::Continue
    }
}
