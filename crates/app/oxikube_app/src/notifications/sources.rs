//! The other notification sources (E14-S07): port-forward drops, updates and agent permission
//! requests reach the centre through [`NotificationSourcePort`], which [`NotificationService`]
//! implements. Each hook builds a [`NotificationInput`] with a stable dedupe key
//! (`oxikube_domain::notification` documents the formats) and goes through the same path as a
//! Warning event: the kind switch, the per-cluster mute, the dedupe, the rate limit, the
//! read state and the OS output.
//!
//! | Hook | Notification |
//! |---|---|
//! | port-forward drop | one per drop (Warning, Error once it gave up), one row per forward; a recovery marks it read |
//! | update available | one per version (Info), with the release link |
//! | agent permission requested | Warning that stays (sticky toast) until the request is answered, which marks it read; none while the agent panel is focused |

use std::sync::Arc;

use oxikube_domain::notification::{
    AgentPermissionRequest, ForwardPhase, NotificationKind, PortForwardChange, UpdateAvailable,
    agent_permission_key,
};
use oxikube_ports::{NotificationLevel, NotificationSourcePort};

use super::model::NotificationInput;
use super::service::{NotificationService, State};

impl NotificationService {
    /// The hooks of the other sources as the port the emitting epics hold (E15, E24, E27).
    pub fn sources(&self) -> Arc<dyn NotificationSourcePort> {
        Arc::new(self.clone())
    }

    /// Marks the row with `key` read; returns whether it was unread.
    fn resolve(&self, key: &str) -> bool {
        self.change(|state: &mut State| state.centre.mark_read_by_key(key))
    }
}

impl NotificationSourcePort for NotificationService {
    fn port_forward_changed(&self, change: PortForwardChange) {
        if change.is_recovery() {
            self.resolve(&change.dedupe_key());
            return;
        }
        if !change.is_drop() {
            return;
        }
        let failed = change.to == ForwardPhase::Failed;
        let (title, level, fallback) = if failed {
            (
                format!("Port-forward failed: {}", change.label),
                NotificationLevel::Error,
                "The forward stopped and will not reconnect by itself.",
            )
        } else {
            (
                format!("Port-forward dropped: {}", change.label),
                NotificationLevel::Warning,
                "Connection lost; reconnecting.",
            )
        };
        self.submit_if(
            NotificationInput {
                kind: NotificationKind::PortForward,
                cluster: Some(change.cluster.clone()),
                level,
                title,
                body: change.reason.clone().unwrap_or_else(|| fallback.to_owned()),
                target: change.target.clone(),
                link: None,
                dedupe_key: Arc::from(change.dedupe_key()),
                group: None,
                occurrences: 1,
            },
            |_| true,
        );
    }

    fn update_available(&self, update: UpdateAvailable) {
        let version = update.version.trim().to_owned();
        if version.is_empty() {
            return;
        }
        let body = match update.summary.as_deref().map(str::trim) {
            Some(summary) if !summary.is_empty() => summary.to_owned(),
            _ => format!("Version {version} is ready to download."),
        };
        let input = NotificationInput {
            kind: NotificationKind::UpdateAvailable,
            cluster: None,
            level: NotificationLevel::Info,
            title: format!("Update available: Oxikube {version}"),
            body,
            target: None,
            link: update.link.as_deref().and_then(safe_link),
            dedupe_key: Arc::from(update.dedupe_key()),
            group: None,
            occurrences: 1,
        };
        // Once per version: the version is remembered only when the notification is added, so
        // switching the kind on later still announces it.
        self.submit_if(input, |state| state.announced_versions.insert(version));
    }

    fn agent_permission_requested(&self, request: AgentPermissionRequest) {
        if request.panel_focused {
            return;
        }
        self.submit_if(
            NotificationInput {
                kind: NotificationKind::AgentPermission,
                cluster: request.cluster.clone(),
                level: NotificationLevel::Warning,
                title: format!("{} needs permission", request.agent),
                body: request.summary.clone(),
                target: None,
                link: None,
                dedupe_key: Arc::from(request.dedupe_key()),
                group: None,
                occurrences: 1,
            },
            |_| true,
        );
    }

    fn agent_permission_resolved(&self, session: &str, request_id: &str) {
        self.resolve(&agent_permission_key(session, request_id));
    }
}

/// `link` when it is an `https` URL without whitespace or control characters; a notification
/// never carries a `file:` or `javascript:` link into the browser.
fn safe_link(link: &str) -> Option<Arc<str>> {
    let link = link.trim();
    (link.starts_with("https://")
        && link.len() > "https://".len()
        && !link.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then(|| Arc::from(link))
}
