//! [`FakeNotificationSourcePort`] and the builders of the hooks' inputs (E14-S07): what the
//! port-forward manager (E15), the updater (E24) and the agent panel (E27) need to test their
//! notification hooks without the notification centre.
//!
//! ```
//! use oxikube_ports::NotificationSourcePort as _;
//! use oxikube_testkit::{FakeNotificationSourcePort, SourceCall, emit_portforward_drop};
//!
//! let hooks = FakeNotificationSourcePort::new();
//! emit_portforward_drop(&hooks, "fwd-1");
//! assert!(matches!(hooks.calls()[0], SourceCall::PortForward(_)));
//! ```

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::notification::{
    AgentPermissionRequest, ForwardPhase, PortForwardChange, UpdateAvailable,
};
use oxikube_ports::NotificationSourcePort;
use parking_lot::Mutex;

/// One call made on a [`FakeNotificationSourcePort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceCall {
    /// `port_forward_changed(change)`.
    PortForward(PortForwardChange),
    /// `update_available(update)`.
    Update(UpdateAvailable),
    /// `agent_permission_requested(request)`.
    AgentRequested(AgentPermissionRequest),
    /// `agent_permission_resolved(session, request_id)`.
    AgentResolved {
        /// The session.
        session: String,
        /// The request.
        request_id: String,
    },
}

/// Fake `NotificationSourcePort`: records every call, in order.
#[derive(Debug, Default)]
pub struct FakeNotificationSourcePort {
    calls: Mutex<Vec<SourceCall>>,
}

impl FakeNotificationSourcePort {
    /// A fake with no calls yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The calls made so far, oldest first.
    pub fn calls(&self) -> Vec<SourceCall> {
        self.calls.lock().clone()
    }
}

impl NotificationSourcePort for FakeNotificationSourcePort {
    fn port_forward_changed(&self, change: PortForwardChange) {
        self.calls.lock().push(SourceCall::PortForward(change));
    }

    fn update_available(&self, update: UpdateAvailable) {
        self.calls.lock().push(SourceCall::Update(update));
    }

    fn agent_permission_requested(&self, request: AgentPermissionRequest) {
        self.calls.lock().push(SourceCall::AgentRequested(request));
    }

    fn agent_permission_resolved(&self, session: &str, request_id: &str) {
        self.calls.lock().push(SourceCall::AgentResolved {
            session: session.to_owned(),
            request_id: request_id.to_owned(),
        });
    }
}

/// The cluster the builders use (`kind-oxikube`).
pub fn hook_cluster() -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube"))
}

/// A change of forward `forward_id` from `from` to `to`.
pub fn portforward_change(
    forward_id: &str,
    from: ForwardPhase,
    to: ForwardPhase,
) -> PortForwardChange {
    PortForwardChange {
        cluster: hook_cluster(),
        forward_id: forward_id.to_owned(),
        label: format!("pod/{forward_id} 8080:80"),
        target: None,
        from,
        to,
        reason: Some("connection reset".to_owned()),
    }
}

/// Emits the drop of forward `forward_id` (active, then reconnecting) on `hooks`.
pub fn emit_portforward_drop(hooks: &dyn NotificationSourcePort, forward_id: &str) {
    hooks.port_forward_changed(portforward_change(
        forward_id,
        ForwardPhase::Active,
        ForwardPhase::Reconnecting,
    ));
}

/// Emits the recovery of forward `forward_id` (reconnecting, then active) on `hooks`.
pub fn emit_portforward_recovery(hooks: &dyn NotificationSourcePort, forward_id: &str) {
    hooks.port_forward_changed(portforward_change(
        forward_id,
        ForwardPhase::Reconnecting,
        ForwardPhase::Active,
    ));
}

/// Emits "`version` is available" on `hooks`.
pub fn emit_update_available(hooks: &dyn NotificationSourcePort, version: &str) {
    hooks.update_available(UpdateAvailable {
        version: version.to_owned(),
        link: Some(format!("https://example.com/oxikube/releases/{version}")),
        summary: Some("Faster tables, fewer crashes".to_owned()),
    });
}

/// A permission request `request_id` of `session`, the agent panel not focused.
pub fn agent_permission(session: &str, request_id: &str) -> AgentPermissionRequest {
    AgentPermissionRequest {
        session: session.to_owned(),
        request_id: request_id.to_owned(),
        agent: "Claude".to_owned(),
        summary: "Scale deployment web to 0 replicas".to_owned(),
        cluster: Some(hook_cluster()),
        panel_focused: false,
    }
}

/// Emits the permission request `request_id` of `session` on `hooks` (panel not focused).
pub fn emit_agent_permission(hooks: &dyn NotificationSourcePort, session: &str, request_id: &str) {
    hooks.agent_permission_requested(agent_permission(session, request_id));
}
