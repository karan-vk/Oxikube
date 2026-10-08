//! Health reports: [`SessionHealth`], the [`HealthReporter`] the manager hands to the connector,
//! and how a report moves the session.
//!
//! `Healthy` and `Unhealthy` move `Ready` ↔ `Degraded`. `Failed` is routed by its cause
//! (E06-F440): a non-retryable `Auth` (revoked or expired credentials) goes to `AuthRequired`,
//! where the user signs in again; a transient cause goes to `Error` and starts the automatic
//! reconnect (`reconnect`); anything else goes to `Error` and waits for the user.

use std::sync::{Arc, Weak};

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{SessionEvent, SessionPhase};
use oxikube_ports::{HealthReporter, HealthSignal};

use super::manager::Shared;
use super::reconnect::is_transient;

/// Reports for one connection of one session.
///
/// Holds the manager weakly (an adapter's liveness loop must not keep it alive) and the
/// generation of the attempt it was made for, so reports from a connection that has
/// since been replaced or dropped are ignored.
pub(super) struct SessionHealth {
    pub(super) shared: Weak<Shared>,
    pub(super) cluster: ClusterId,
    pub(super) generation: u64,
}

impl HealthReporter for SessionHealth {
    fn report(&self, signal: HealthSignal) {
        if let Some(shared) = self.shared.upgrade() {
            shared.on_health(&self.cluster, Some(self.generation), signal);
        }
    }
}

impl Shared {
    /// Applies a health signal to a connected session. `generation` is the connection
    /// the report is about (`None`: whatever is connected now).
    pub(super) fn on_health(
        self: &Arc<Self>,
        cluster: &ClusterId,
        generation: Option<u64>,
        signal: HealthSignal,
    ) -> bool {
        let Some(entry) = self.entry(cluster) else {
            return false;
        };
        let released = {
            let mut e = entry.lock();
            let current = generation.is_none_or(|g| g == e.generation);
            if !current || !e.phase().is_connected() {
                tracing::trace!(%cluster, ?signal, "health report ignored");
                return false;
            }
            let Ok(released) = e.apply(session_event(&signal), &self.updates) else {
                return false;
            };
            if let HealthSignal::Failed {
                kind, retryable, ..
            } = signal
                && e.phase() == SessionPhase::Error
                && is_transient(kind, retryable)
            {
                self.schedule_reconnect(&mut e);
            }
            released
        };
        drop(released);
        true
    }
}

/// The session event for `signal`: a `Failed` whose credentials were rejected for good asks for
/// them (`AuthRequired`, with the message alone, as a failed connect would show it).
fn session_event(signal: &HealthSignal) -> SessionEvent {
    match signal {
        HealthSignal::Failed {
            reason,
            kind: ErrorKind::Auth,
            retryable: false,
        } => {
            let label = format!("{}: ", ErrorKind::Auth);
            SessionEvent::AuthNeeded {
                reason: reason.strip_prefix(&label).unwrap_or(reason).to_owned(),
            }
        }
        other => other.to_session_event(),
    }
}
