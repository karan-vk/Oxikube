//! [`NotificationSourcePort`]: the hooks through which the other epics tell the notification
//! centre something (E14-S07).
//!
//! # Implementor
//!
//! Implemented by `oxikube_app::NotificationService` (the notification centre); the testkit ships
//! `FakeNotificationSourcePort`, which records the calls. Unlike the other ports this one points
//! *into* the app: the port-forward manager (E15), the updater (E24) and the agent panel (E27)
//! hold an `Arc<dyn NotificationSourcePort>` and call it, so they never depend on the app crate
//! and the centre never depends on theirs. Each call carries a plain domain type
//! (`oxikube_domain::notification`) whose dedupe key makes repeats update one notification.
//!
//! The methods are synchronous and return at once (no I/O, nothing blocks): they may be called
//! from any thread, including a UI callback. A call that is filtered out (the kind or the
//! cluster is muted, the change is not a drop, the version was announced) is dropped silently.

use oxikube_domain::notification::{AgentPermissionRequest, PortForwardChange, UpdateAvailable};

/// The hooks the emitting epics call. See the [module docs](self).
pub trait NotificationSourcePort: Send + Sync {
    /// A port-forward changed state. Only a drop (see
    /// [`PortForwardChange::is_drop`]) raises a notification, once per drop, deduplicated per
    /// forward; every other change is ignored. A recovery marks the forward's drop notification
    /// read.
    fn port_forward_changed(&self, change: PortForwardChange);

    /// A newer version is available. Notifies once per version, however often it is reported.
    fn update_available(&self, update: UpdateAvailable);

    /// An agent asked for permission. Notifies unless the agent panel is focused; the
    /// notification stays (its toast does not time out) until
    /// [`agent_permission_resolved`](Self::agent_permission_resolved).
    fn agent_permission_requested(&self, request: AgentPermissionRequest);

    /// The permission request `request_id` of `session` was answered (or withdrawn): its
    /// notification is marked read.
    fn agent_permission_resolved(&self, session: &str, request_id: &str);
}
