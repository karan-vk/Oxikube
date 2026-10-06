//! The mutable record behind one session, and the one place its state changes.
//!
//! An [`Entry`] lives behind its own `parking_lot::Mutex`; every critical section is
//! short and never spans an `.await`. All state moves go through [`Entry::apply`], which
//! runs the pure domain transition, keeps the invariants below and sends the updates
//! while the lock is held, so one session's updates are ordered.
//!
//! # Invariants
//!
//! * `connection` is `Some` exactly when the phase is `Ready` or `Degraded`.
//! * `capabilities` is empty unless connected.
//! * `abort` is `Some` only while `Connecting`.
//! * `generation` changes on every new attempt and whenever a connection is released, so
//!   late results and health reports of an older connection are recognised and ignored.

use std::sync::Arc;

use futures::future::AbortHandle;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::redact::redact;
use oxikube_domain::session::{
    ClusterSessionState, InvalidTransition, NamespaceSelection, SessionEvent, SessionPhase,
};
use oxikube_domain::{Capabilities, ClusterColour};
use oxikube_ports::{ClusterConnection, ClusterPrefs, ExecInteractivity};

use super::config::SessionOptions;
use super::model::ClusterSession;
use super::updates::{SessionChange, UpdateSender};

/// A connection taken out of an entry. Drop it *after* releasing the entry lock: an
/// adapter's teardown may report health, which takes the same lock.
#[must_use = "drop the released connection outside the entry lock"]
pub(super) struct Released(#[allow(dead_code)] pub(super) Option<ClusterConnection>);

pub(super) struct Entry {
    pub(super) id: ClusterId,
    pub(super) context: ContextName,
    pub(super) state: ClusterSessionState,
    pub(super) generation: u64,
    pub(super) connection: Option<ClusterConnection>,
    pub(super) capabilities: Capabilities,
    pub(super) namespace_selection: NamespaceSelection,
    pub(super) read_only: bool,
    pub(super) colour: Option<ClusterColour>,
    pub(super) exec_interactivity: ExecInteractivity,
    pub(super) display_name: Option<String>,
    /// The settings last applied; a new push is applied as a delta against them.
    pub(super) prefs: Arc<ClusterPrefs>,
    pub(super) abort: Option<AbortHandle>,
}

impl Entry {
    pub(super) fn new(id: ClusterId, context: ContextName, options: SessionOptions) -> Self {
        Self {
            id,
            context,
            state: ClusterSessionState::Disconnected,
            generation: 0,
            connection: None,
            capabilities: Capabilities::empty(),
            namespace_selection: options.namespace_selection,
            read_only: options.read_only,
            colour: options.colour,
            exec_interactivity: options.exec_interactivity,
            display_name: options.display_name,
            prefs: options.prefs,
            abort: None,
        }
    }

    pub(super) fn snapshot(&self) -> ClusterSession {
        ClusterSession {
            id: self.id.clone(),
            context: self.context.clone(),
            state: self.state.clone(),
            capabilities: self.capabilities,
            namespace_selection: self.namespace_selection.clone(),
            read_only: self.read_only,
            colour: self.colour,
            exec_interactivity: self.exec_interactivity,
            display_name: self.display_name.clone(),
            prefs: self.prefs.clone(),
            ports: self.connection.as_ref().map(|c| c.ports.clone()),
        }
    }

    pub(super) fn phase(&self) -> SessionPhase {
        self.state.phase()
    }

    /// Feeds `event` to the state machine.
    ///
    /// On success the new state is stored and announced (unless it equals the old one).
    /// `Connect` starts a new generation. Leaving the connected phases releases the
    /// connection and clears the capabilities. A `Connected` event must be preceded by
    /// [`install`](Self::install). Reasons are redacted here, whatever their source.
    pub(super) fn apply(
        &mut self,
        event: SessionEvent,
        updates: &UpdateSender,
    ) -> Result<Released, InvalidTransition> {
        let event = match event {
            SessionEvent::AuthNeeded { reason } => SessionEvent::AuthNeeded {
                reason: redact(&reason).into_owned(),
            },
            SessionEvent::Failed { reason } => SessionEvent::Failed {
                reason: redact(&reason).into_owned(),
            },
            other => other,
        };
        let is_connect = matches!(event, SessionEvent::Connect);
        let from = self.phase();
        let next = self.state.clone().transition(event)?;
        if is_connect {
            self.generation += 1;
        }
        let mut released = None;
        if !next.phase().is_connected() {
            self.abort = self
                .abort
                .take()
                .filter(|_| next.phase() == SessionPhase::Connecting);
            if let Some(connection) = self.connection.take() {
                self.generation += 1;
                released = Some(connection);
            }
            self.set_capabilities(Capabilities::empty(), updates);
        }
        if next != self.state {
            self.state = next.clone();
            updates.send(&self.id, SessionChange::StateChanged { from, state: next });
        }
        Ok(Released(released))
    }

    /// Stores the connection and capabilities of a successful attempt, ahead of the
    /// `Connected` event (so `Ready` listeners already see the capabilities).
    pub(super) fn install(
        &mut self,
        connection: ClusterConnection,
        capabilities: Capabilities,
        updates: &UpdateSender,
    ) {
        self.abort = None;
        self.connection = Some(connection);
        self.set_capabilities(capabilities, updates);
    }

    /// Takes the connection and moves to `Disconnected` from wherever the session is.
    /// Aborts an in-flight attempt. A no-op when already disconnected.
    pub(super) fn disconnect(&mut self, updates: &UpdateSender) -> Released {
        if let Some(abort) = self.abort.take() {
            abort.abort();
        }
        if self.phase() == SessionPhase::Disconnected {
            return Released(None);
        }
        // Bump even when no connection is held (Connecting, AuthRequired, Error), so a
        // result of the aborted attempt that is already on its way is ignored.
        self.generation += 1;
        self.apply(SessionEvent::Disconnect, updates)
            .unwrap_or(Released(None))
    }

    fn set_capabilities(&mut self, capabilities: Capabilities, updates: &UpdateSender) {
        if self.capabilities != capabilities {
            self.capabilities = capabilities;
            updates.send(&self.id, SessionChange::CapabilitiesChanged(capabilities));
        }
    }
}
