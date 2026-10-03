//! Cluster session state machine and namespace selection.
//!
//! A cluster tab (a `ClusterSession`, owned by `oxikube_app`) is always in
//! exactly one [`ClusterSessionState`]. The session manager feeds it
//! [`SessionEvent`]s and [`ClusterSessionState::transition`] either returns the
//! next state or an [`InvalidTransition`]; it never panics and never reads a
//! clock.
//!
//! The second half of the module is the namespace selection:
//! [`NamespaceSelection`] (`All` or a `Set`), [`NamespaceFavourites`] and
//! [`WatchScope`], which turns a selection plus a kind's [`Scope`] into the
//! shape of the feeds to open.
//!
//! # Transition table
//!
//! ```text
//! state         event        -> next
//! Disconnected  Connect      -> Connecting
//! Connecting    Connected    -> Ready
//! Connecting    AuthNeeded   -> AuthRequired
//! Connecting    Failed       -> Error
//! Connecting    Disconnect   -> Disconnected   (cancel)
//! AuthRequired  Connect      -> Connecting     (credentials supplied)
//! AuthRequired  Disconnect   -> Disconnected   (give up)
//! Ready         Healthy      -> Ready          (probe ok, no change)
//! Ready         Unhealthy    -> Degraded
//! Ready         Failed       -> Error
//! Ready         Disconnect   -> Disconnected
//! Degraded      Healthy      -> Ready          (recovered)
//! Degraded      Unhealthy    -> Degraded       (still failing)
//! Degraded      Failed       -> Error
//! Degraded      Disconnect   -> Disconnected
//! Error         Connect      -> Connecting     (retry)
//! Error         Disconnect   -> Disconnected   (close)
//! ```
//!
//! Every pair not listed is rejected. Notable decisions:
//!
//! * `Disconnect` is legal from every state except `Disconnected` itself: a
//!   second disconnect is a caller bug, not a no-op.
//! * A health probe emits `Healthy` / `Unhealthy` on every tick, so
//!   `Ready + Healthy` and `Degraded + Unhealthy` are legal self-transitions.
//! * `Ready -> Error` is allowed directly (the connection died between probes)
//!   in addition to the `Ready -> Degraded -> Error` path.
//! * Reasons are plain strings for now (an `ErrorKind` can join them once the
//!   error taxonomy is serialisable) and must never contain secrets.

use std::collections::BTreeSet;
use std::fmt;

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::ids::Scope;

/// The discriminant of a [`ClusterSessionState`], without its reason payload.
///
/// Use it for table lookups, logging and UI branching where the reason is
/// irrelevant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    /// No connection and none being attempted.
    Disconnected,
    /// A connection attempt is in flight.
    Connecting,
    /// The cluster needs credentials the user has to supply.
    AuthRequired,
    /// Connected and healthy.
    Ready,
    /// Connected but the health probe is failing.
    Degraded,
    /// The connection failed and is not being retried automatically.
    Error,
}

impl SessionPhase {
    /// Every phase, in lifecycle order.
    pub const ALL: [SessionPhase; 6] = [
        SessionPhase::Disconnected,
        SessionPhase::Connecting,
        SessionPhase::AuthRequired,
        SessionPhase::Ready,
        SessionPhase::Degraded,
        SessionPhase::Error,
    ];

    /// Stable variant name, e.g. `"AuthRequired"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionPhase::Disconnected => "Disconnected",
            SessionPhase::Connecting => "Connecting",
            SessionPhase::AuthRequired => "AuthRequired",
            SessionPhase::Ready => "Ready",
            SessionPhase::Degraded => "Degraded",
            SessionPhase::Error => "Error",
        }
    }

    /// The phase reached from `self` on `event`, or `None` when the move is illegal.
    ///
    /// This is the single source of truth for the transition table in the
    /// module docs; [`ClusterSessionState::transition`] is built on it.
    pub const fn next(self, event: SessionEventKind) -> Option<SessionPhase> {
        use SessionEventKind as E;
        use SessionPhase as P;
        match (self, event) {
            (P::Disconnected, E::Connect) => Some(P::Connecting),
            (P::Connecting, E::Connected) => Some(P::Ready),
            (P::Connecting, E::AuthNeeded) => Some(P::AuthRequired),
            (P::Connecting, E::Failed) => Some(P::Error),
            (P::AuthRequired, E::Connect) => Some(P::Connecting),
            (P::Ready, E::Healthy) => Some(P::Ready),
            (P::Ready, E::Unhealthy) => Some(P::Degraded),
            (P::Ready, E::Failed) => Some(P::Error),
            (P::Degraded, E::Healthy) => Some(P::Ready),
            (P::Degraded, E::Unhealthy) => Some(P::Degraded),
            (P::Degraded, E::Failed) => Some(P::Error),
            (P::Error, E::Connect) => Some(P::Connecting),
            (P::Disconnected, E::Disconnect) => None,
            (_, E::Disconnect) => Some(P::Disconnected),
            _ => None,
        }
    }

    /// Whether `event` is legal in this phase.
    pub const fn allows(self, event: SessionEventKind) -> bool {
        self.next(event).is_some()
    }

    /// Whether the session has a live connection to the cluster.
    pub const fn is_connected(self) -> bool {
        matches!(self, SessionPhase::Ready | SessionPhase::Degraded)
    }
}

impl fmt::Display for SessionPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The discriminant of a [`SessionEvent`], without its reason payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    /// Start or retry a connection.
    Connect,
    /// The connection attempt succeeded.
    Connected,
    /// The cluster needs credentials.
    AuthNeeded,
    /// The health probe succeeded.
    Healthy,
    /// The health probe failed.
    Unhealthy,
    /// The connection failed for good.
    Failed,
    /// Close, cancel or give up.
    Disconnect,
}

impl SessionEventKind {
    /// Every event kind.
    pub const ALL: [SessionEventKind; 7] = [
        SessionEventKind::Connect,
        SessionEventKind::Connected,
        SessionEventKind::AuthNeeded,
        SessionEventKind::Healthy,
        SessionEventKind::Unhealthy,
        SessionEventKind::Failed,
        SessionEventKind::Disconnect,
    ];

    /// Stable variant name, e.g. `"AuthNeeded"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionEventKind::Connect => "Connect",
            SessionEventKind::Connected => "Connected",
            SessionEventKind::AuthNeeded => "AuthNeeded",
            SessionEventKind::Healthy => "Healthy",
            SessionEventKind::Unhealthy => "Unhealthy",
            SessionEventKind::Failed => "Failed",
            SessionEventKind::Disconnect => "Disconnect",
        }
    }
}

impl fmt::Display for SessionEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Something that happened to a session, fed to [`ClusterSessionState::transition`].
///
/// Reasons are user-visible text. They must not contain tokens, passwords or
/// Secret data; adapters redact before building them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEvent {
    /// Start a connection, retry after an error, or resume after credentials arrived.
    Connect,
    /// The connection attempt succeeded.
    Connected,
    /// The cluster needs credentials (exec plugin login, expired token, ...).
    AuthNeeded {
        /// Why credentials are needed.
        reason: String,
    },
    /// The health probe succeeded.
    Healthy,
    /// The health probe failed but the session may recover.
    Unhealthy,
    /// The connection failed and will not recover without a retry.
    Failed {
        /// What went wrong.
        reason: String,
    },
    /// Cancel, give up or close the session.
    Disconnect,
}

impl SessionEvent {
    /// The event's discriminant.
    pub const fn kind(&self) -> SessionEventKind {
        match self {
            SessionEvent::Connect => SessionEventKind::Connect,
            SessionEvent::Connected => SessionEventKind::Connected,
            SessionEvent::AuthNeeded { .. } => SessionEventKind::AuthNeeded,
            SessionEvent::Healthy => SessionEventKind::Healthy,
            SessionEvent::Unhealthy => SessionEventKind::Unhealthy,
            SessionEvent::Failed { .. } => SessionEventKind::Failed,
            SessionEvent::Disconnect => SessionEventKind::Disconnect,
        }
    }
}

/// An event that is not legal in the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
#[error("session event {event} is not allowed while {from}")]
pub struct InvalidTransition {
    /// The phase the session was in.
    pub from: SessionPhase,
    /// The rejected event.
    pub event: SessionEventKind,
}

/// Connection state of one cluster session.
///
/// `AuthRequired` and `Error` carry a reason to show the user. Move between
/// states with [`transition`](Self::transition); see the module docs for the
/// full table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClusterSessionState {
    /// No connection and none being attempted. The initial state.
    #[default]
    Disconnected,
    /// A connection attempt is in flight.
    Connecting,
    /// The cluster needs credentials the user has to supply.
    AuthRequired {
        /// Why credentials are needed.
        reason: String,
    },
    /// Connected and healthy.
    Ready,
    /// Connected but the health probe is failing.
    Degraded,
    /// The connection failed.
    Error {
        /// What went wrong.
        reason: String,
    },
}

impl ClusterSessionState {
    /// The state's discriminant.
    pub const fn phase(&self) -> SessionPhase {
        match self {
            ClusterSessionState::Disconnected => SessionPhase::Disconnected,
            ClusterSessionState::Connecting => SessionPhase::Connecting,
            ClusterSessionState::AuthRequired { .. } => SessionPhase::AuthRequired,
            ClusterSessionState::Ready => SessionPhase::Ready,
            ClusterSessionState::Degraded => SessionPhase::Degraded,
            ClusterSessionState::Error { .. } => SessionPhase::Error,
        }
    }

    /// The reason carried by `AuthRequired` and `Error`, if any.
    pub fn reason(&self) -> Option<&str> {
        match self {
            ClusterSessionState::AuthRequired { reason }
            | ClusterSessionState::Error { reason } => Some(reason),
            _ => None,
        }
    }

    /// Whether `event` would be accepted in this state.
    pub const fn can_handle(&self, event: SessionEventKind) -> bool {
        self.phase().allows(event)
    }

    /// Apply `event` and return the next state.
    ///
    /// # Errors
    ///
    /// [`InvalidTransition`] when the pair is not in the transition table. The
    /// caller keeps its current state in that case.
    pub fn transition(self, event: SessionEvent) -> Result<ClusterSessionState, InvalidTransition> {
        let from = self.phase();
        if !from.allows(event.kind()) {
            return Err(InvalidTransition {
                from,
                event: event.kind(),
            });
        }
        // Once legal, the target is fixed by the event alone; the table in
        // `SessionPhase::next` only decides legality (a test keeps both in step).
        Ok(match event {
            SessionEvent::Connect => ClusterSessionState::Connecting,
            SessionEvent::Connected | SessionEvent::Healthy => ClusterSessionState::Ready,
            SessionEvent::AuthNeeded { reason } => ClusterSessionState::AuthRequired { reason },
            SessionEvent::Unhealthy => ClusterSessionState::Degraded,
            SessionEvent::Failed { reason } => ClusterSessionState::Error { reason },
            SessionEvent::Disconnect => ClusterSessionState::Disconnected,
        })
    }
}

impl fmt::Display for ClusterSessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason() {
            Some(reason) => write!(f, "{}: {reason}", self.phase()),
            None => f.write_str(self.phase().as_str()),
        }
    }
}

/// Which namespaces a session watches.
///
/// `All` means one cluster-wide watch per kind. `Set` means one namespaced
/// watch per namespace, which is what an RBAC-limited user needs.
///
/// # Rules
///
/// * A set is never empty and never holds an empty name: an empty or
///   all-blank input normalises to `All`, blank names are dropped, surrounding
///   whitespace is trimmed. This holds for every constructor, mutator and for
///   deserialisation, so the invariant cannot be bypassed.
/// * Names are deduplicated and kept in sorted order (`BTreeSet`).
/// * Removing the last namespace of a set yields `All`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "NamespaceSelectionRepr", rename_all = "snake_case")]
pub enum NamespaceSelection {
    /// Every namespace (one cluster-wide watch). The default.
    #[default]
    All,
    /// A non-empty, sorted set of namespace names.
    Set(BTreeSet<String>),
}

/// Wire shape of [`NamespaceSelection`]; deserialising through it re-applies the rules.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum NamespaceSelectionRepr {
    All,
    Set(Vec<String>),
}

impl From<NamespaceSelectionRepr> for NamespaceSelection {
    fn from(repr: NamespaceSelectionRepr) -> Self {
        match repr {
            NamespaceSelectionRepr::All => NamespaceSelection::All,
            NamespaceSelectionRepr::Set(names) => NamespaceSelection::from_names(names),
        }
    }
}

impl NamespaceSelection {
    /// Build a selection from names, applying the normalisation rules.
    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let set: BTreeSet<String> = names
            .into_iter()
            .map(|n| n.as_ref().trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect();
        if set.is_empty() {
            NamespaceSelection::All
        } else {
            NamespaceSelection::Set(set)
        }
    }

    /// A selection of exactly one namespace (`All` if the name is blank).
    pub fn single(name: impl AsRef<str>) -> Self {
        Self::from_names([name])
    }

    /// Whether this is `All`.
    pub fn is_all(&self) -> bool {
        matches!(self, NamespaceSelection::All)
    }

    /// Whether `namespace` is selected. `All` selects everything.
    pub fn contains(&self, namespace: &str) -> bool {
        match self {
            NamespaceSelection::All => true,
            NamespaceSelection::Set(set) => set.contains(namespace),
        }
    }

    /// The selected names in sorted order; empty for `All`.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        let set = match self {
            NamespaceSelection::All => None,
            NamespaceSelection::Set(set) => Some(set),
        };
        set.into_iter().flatten().map(String::as_str)
    }

    /// Select `namespace`. Adding to `All` narrows it to just that namespace.
    ///
    /// Returns whether the selection changed. Blank names are ignored.
    pub fn insert(&mut self, namespace: &str) -> bool {
        let namespace = namespace.trim();
        if namespace.is_empty() {
            return false;
        }
        match self {
            NamespaceSelection::All => {
                *self = NamespaceSelection::single(namespace);
                true
            }
            NamespaceSelection::Set(set) => set.insert(namespace.to_owned()),
        }
    }

    /// Deselect `namespace`. Removing the last one yields `All`; removing from
    /// `All` does nothing.
    ///
    /// Returns whether the selection changed.
    pub fn remove(&mut self, namespace: &str) -> bool {
        let NamespaceSelection::Set(set) = self else {
            return false;
        };
        let removed = set.remove(namespace.trim());
        if set.is_empty() {
            *self = NamespaceSelection::All;
        }
        removed
    }
}

/// The user's favourite namespaces: an insertion-ordered set of names.
///
/// Owned by the session (and persisted through settings later), not by
/// [`NamespaceSelection`], so a favourite survives switching the selection.
/// Blank names are ignored, duplicates keep their first position.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<String>", into = "Vec<String>")]
pub struct NamespaceFavourites(IndexSet<String>);

impl From<Vec<String>> for NamespaceFavourites {
    fn from(names: Vec<String>) -> Self {
        names.iter().map(String::as_str).collect()
    }
}

impl From<NamespaceFavourites> for Vec<String> {
    fn from(favourites: NamespaceFavourites) -> Self {
        favourites.0.into_iter().collect()
    }
}

impl<'a> FromIterator<&'a str> for NamespaceFavourites {
    fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
        let mut favourites = NamespaceFavourites::default();
        for name in iter {
            favourites.add(name);
        }
        favourites
    }
}

impl NamespaceFavourites {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `namespace` unless it is blank or already present.
    ///
    /// Returns whether it was added.
    pub fn add(&mut self, namespace: &str) -> bool {
        let namespace = namespace.trim();
        !namespace.is_empty() && self.0.insert(namespace.to_owned())
    }

    /// Remove `namespace`, keeping the order of the rest. Returns whether it was present.
    pub fn remove(&mut self, namespace: &str) -> bool {
        self.0.shift_remove(namespace.trim())
    }

    /// Add the namespace if absent, remove it if present. Returns whether it is now a favourite.
    pub fn toggle(&mut self, namespace: &str) -> bool {
        if self.remove(namespace) {
            false
        } else {
            self.add(namespace)
        }
    }

    /// Whether `namespace` is a favourite.
    pub fn contains(&self, namespace: &str) -> bool {
        self.0.contains(namespace.trim())
    }

    /// The favourites in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// Number of favourites.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no favourites.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The shape of the watches to open for one kind under a [`NamespaceSelection`].
///
/// Derived when the selection changes, not per event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchScope {
    /// One cluster-wide watch.
    Cluster,
    /// One namespaced watch per name. Never empty; sorted and unique.
    Namespaces(Vec<String>),
}

impl WatchScope {
    /// Derive the watch scope for a kind of the given [`Scope`].
    ///
    /// Cluster-scoped kinds (`Node`, `Namespace`, ...) ignore the selection and
    /// always get [`WatchScope::Cluster`]. For namespaced kinds `All` gives
    /// `Cluster` and a set gives `Namespaces` with the names in sorted order.
    pub fn derive(selection: &NamespaceSelection, kind_scope: Scope) -> Self {
        match (kind_scope, selection) {
            (Scope::Namespaced, NamespaceSelection::Set(set)) => {
                WatchScope::Namespaces(set.iter().cloned().collect())
            }
            _ => WatchScope::Cluster,
        }
    }

    /// How many independent watches this scope opens.
    pub fn watch_count(&self) -> usize {
        match self {
            WatchScope::Cluster => 1,
            WatchScope::Namespaces(names) => names.len(),
        }
    }

    /// The namespaces to watch; empty for `Cluster`.
    pub fn namespaces(&self) -> &[String] {
        match self {
            WatchScope::Cluster => &[],
            WatchScope::Namespaces(names) => names,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn event_for(kind: SessionEventKind) -> SessionEvent {
        match kind {
            SessionEventKind::Connect => SessionEvent::Connect,
            SessionEventKind::Connected => SessionEvent::Connected,
            SessionEventKind::AuthNeeded => SessionEvent::AuthNeeded {
                reason: "login".into(),
            },
            SessionEventKind::Healthy => SessionEvent::Healthy,
            SessionEventKind::Unhealthy => SessionEvent::Unhealthy,
            SessionEventKind::Failed => SessionEvent::Failed {
                reason: "boom".into(),
            },
            SessionEventKind::Disconnect => SessionEvent::Disconnect,
        }
    }

    fn state_for(phase: SessionPhase) -> ClusterSessionState {
        match phase {
            SessionPhase::Disconnected => ClusterSessionState::Disconnected,
            SessionPhase::Connecting => ClusterSessionState::Connecting,
            SessionPhase::AuthRequired => ClusterSessionState::AuthRequired {
                reason: "expired".into(),
            },
            SessionPhase::Ready => ClusterSessionState::Ready,
            SessionPhase::Degraded => ClusterSessionState::Degraded,
            SessionPhase::Error => ClusterSessionState::Error {
                reason: "down".into(),
            },
        }
    }

    /// The documented table, written independently of `SessionPhase::next`.
    const ALLOWED: &[(SessionPhase, SessionEventKind, SessionPhase)] = {
        use SessionEventKind as E;
        use SessionPhase as P;
        &[
            (P::Disconnected, E::Connect, P::Connecting),
            (P::Connecting, E::Connected, P::Ready),
            (P::Connecting, E::AuthNeeded, P::AuthRequired),
            (P::Connecting, E::Failed, P::Error),
            (P::Connecting, E::Disconnect, P::Disconnected),
            (P::AuthRequired, E::Connect, P::Connecting),
            (P::AuthRequired, E::Disconnect, P::Disconnected),
            (P::Ready, E::Healthy, P::Ready),
            (P::Ready, E::Unhealthy, P::Degraded),
            (P::Ready, E::Failed, P::Error),
            (P::Ready, E::Disconnect, P::Disconnected),
            (P::Degraded, E::Healthy, P::Ready),
            (P::Degraded, E::Unhealthy, P::Degraded),
            (P::Degraded, E::Failed, P::Error),
            (P::Degraded, E::Disconnect, P::Disconnected),
            (P::Error, E::Connect, P::Connecting),
            (P::Error, E::Disconnect, P::Disconnected),
        ]
    };

    #[test]
    fn every_state_event_pair_matches_the_table() {
        let mut checked = 0;
        for phase in SessionPhase::ALL {
            for kind in SessionEventKind::ALL {
                checked += 1;
                let expected = ALLOWED
                    .iter()
                    .find(|(f, e, _)| *f == phase && *e == kind)
                    .map(|(_, _, to)| *to);
                let result = state_for(phase).transition(event_for(kind));
                match expected {
                    Some(to) => {
                        let next = result.unwrap_or_else(|e| panic!("{phase} + {kind}: {e}"));
                        assert_eq!(next.phase(), to, "{phase} + {kind}");
                        assert_eq!(phase.next(kind), Some(to));
                        assert!(state_for(phase).can_handle(kind));
                    }
                    None => {
                        assert_eq!(
                            result,
                            Err(InvalidTransition {
                                from: phase,
                                event: kind
                            }),
                            "{phase} + {kind} must be rejected"
                        );
                        assert!(!phase.allows(kind));
                    }
                }
            }
        }
        assert_eq!(checked, 42);
        assert_eq!(ALLOWED.len(), 17);
    }

    #[test]
    fn disconnect_is_legal_from_every_state_but_disconnected() {
        for phase in SessionPhase::ALL {
            let result = state_for(phase).transition(SessionEvent::Disconnect);
            if phase == SessionPhase::Disconnected {
                assert!(result.is_err());
            } else {
                assert_eq!(result, Ok(ClusterSessionState::Disconnected));
            }
        }
    }

    fn run(
        start: ClusterSessionState,
        events: impl IntoIterator<Item = SessionEvent>,
    ) -> Vec<SessionPhase> {
        let mut state = start;
        let mut path = vec![state.phase()];
        for event in events {
            state = state.transition(event).expect("legal path");
            path.push(state.phase());
        }
        path
    }

    #[test]
    fn happy_connect_path() {
        use SessionPhase as P;
        let path = run(
            ClusterSessionState::default(),
            [SessionEvent::Connect, SessionEvent::Connected],
        );
        assert_eq!(path, [P::Disconnected, P::Connecting, P::Ready]);
    }

    #[test]
    fn connect_with_auth_required_then_success() {
        let mut state = ClusterSessionState::Disconnected;
        state = state.transition(SessionEvent::Connect).unwrap();
        state = state
            .transition(SessionEvent::AuthNeeded {
                reason: "exec plugin needs login".into(),
            })
            .unwrap();
        assert_eq!(state.reason(), Some("exec plugin needs login"));
        state = state.transition(SessionEvent::Connect).unwrap();
        assert_eq!(state, ClusterSessionState::Connecting);
        assert_eq!(state.reason(), None);
        state = state.transition(SessionEvent::Connected).unwrap();
        assert_eq!(state, ClusterSessionState::Ready);
    }

    #[test]
    fn degraded_then_recovered() {
        use SessionPhase as P;
        let path = run(
            ClusterSessionState::Ready,
            [
                SessionEvent::Unhealthy,
                SessionEvent::Unhealthy,
                SessionEvent::Healthy,
            ],
        );
        assert_eq!(path, [P::Ready, P::Degraded, P::Degraded, P::Ready]);
    }

    #[test]
    fn degraded_then_error_for_bad_token() {
        let state = ClusterSessionState::Ready
            .transition(SessionEvent::Unhealthy)
            .unwrap()
            .transition(SessionEvent::Failed {
                reason: "401 Unauthorized".into(),
            })
            .unwrap();
        assert_eq!(
            state,
            ClusterSessionState::Error {
                reason: "401 Unauthorized".into()
            }
        );
    }

    #[test]
    fn retry_from_error_reconnects_and_clears_reason() {
        use SessionPhase as P;
        let start = ClusterSessionState::Error {
            reason: "refused".into(),
        };
        let path = run(
            start,
            [
                SessionEvent::Connect,
                SessionEvent::Connected,
                SessionEvent::Healthy,
            ],
        );
        assert_eq!(path, [P::Error, P::Connecting, P::Ready, P::Ready]);
    }

    #[test]
    fn cancel_while_connecting_and_give_up_on_auth() {
        assert_eq!(
            ClusterSessionState::Connecting.transition(SessionEvent::Disconnect),
            Ok(ClusterSessionState::Disconnected)
        );
        assert_eq!(
            state_for(SessionPhase::AuthRequired).transition(SessionEvent::Disconnect),
            Ok(ClusterSessionState::Disconnected)
        );
    }

    #[test]
    fn invalid_transition_message_names_both_sides() {
        let err = ClusterSessionState::Disconnected
            .transition(SessionEvent::Healthy)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "session event Healthy is not allowed while Disconnected"
        );
    }

    #[test]
    fn connected_phases() {
        for phase in SessionPhase::ALL {
            let expected = matches!(phase, SessionPhase::Ready | SessionPhase::Degraded);
            assert_eq!(phase.is_connected(), expected);
        }
    }

    #[test]
    fn display_includes_reason() {
        assert_eq!(ClusterSessionState::Ready.to_string(), "Ready");
        assert_eq!(state_for(SessionPhase::Error).to_string(), "Error: down");
    }

    #[test]
    fn state_serde_round_trips() {
        for phase in SessionPhase::ALL {
            let state = state_for(phase);
            let json = serde_json::to_string(&state).unwrap();
            let back: ClusterSessionState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
        assert_eq!(
            serde_json::to_string(&ClusterSessionState::Ready).unwrap(),
            r#"{"state":"ready"}"#
        );
        assert_eq!(
            serde_json::to_string(&state_for(SessionPhase::AuthRequired)).unwrap(),
            r#"{"state":"auth_required","reason":"expired"}"#
        );
    }

    #[test]
    fn event_and_phase_serde_round_trips() {
        for kind in SessionEventKind::ALL {
            let event = event_for(kind);
            assert_eq!(event.kind(), kind);
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<SessionEvent>(&json).unwrap(), event);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(
                serde_json::from_str::<SessionEventKind>(&json).unwrap(),
                kind
            );
        }
        for phase in SessionPhase::ALL {
            let json = serde_json::to_string(&phase).unwrap();
            assert_eq!(serde_json::from_str::<SessionPhase>(&json).unwrap(), phase);
        }
        let err = InvalidTransition {
            from: SessionPhase::Ready,
            event: SessionEventKind::Connect,
        };
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(
            serde_json::from_str::<InvalidTransition>(&json).unwrap(),
            err
        );
    }

    // --- NamespaceSelection ---

    fn set(names: &[&str]) -> NamespaceSelection {
        NamespaceSelection::from_names(names)
    }

    #[test]
    fn empty_set_normalises_to_all() {
        assert_eq!(set(&[]), NamespaceSelection::All);
        assert_eq!(set(&["", "  "]), NamespaceSelection::All);
        assert_eq!(NamespaceSelection::single(""), NamespaceSelection::All);
        assert_eq!(NamespaceSelection::default(), NamespaceSelection::All);
    }

    #[test]
    fn duplicates_collapse_and_order_is_sorted() {
        let sel = set(&["prod", "dev", "prod", " kube-system "]);
        assert_eq!(
            sel.names().collect::<Vec<_>>(),
            ["dev", "kube-system", "prod"]
        );
        assert!(!sel.is_all());
    }

    #[test]
    fn contains_semantics() {
        assert!(NamespaceSelection::All.contains("anything"));
        let sel = set(&["a", "b"]);
        assert!(sel.contains("a"));
        assert!(!sel.contains("c"));
        assert_eq!(NamespaceSelection::All.names().count(), 0);
    }

    #[test]
    fn insert_and_remove_follow_the_rules() {
        let mut sel = NamespaceSelection::All;
        assert!(!sel.insert(" "));
        assert!(sel.is_all());
        assert!(sel.insert("a"));
        assert_eq!(sel, set(&["a"]));
        assert!(!sel.insert("a"));
        assert!(sel.insert("b"));
        assert!(sel.remove("a"));
        assert!(!sel.remove("a"));
        assert_eq!(sel, set(&["b"]));
        assert!(sel.remove("b"));
        assert_eq!(sel, NamespaceSelection::All);
        assert!(!sel.remove("b"));
    }

    #[test]
    fn selection_serde_round_trips_and_normalises() {
        for sel in [NamespaceSelection::All, set(&["a", "b"])] {
            let json = serde_json::to_string(&sel).unwrap();
            assert_eq!(
                serde_json::from_str::<NamespaceSelection>(&json).unwrap(),
                sel
            );
        }
        assert_eq!(
            serde_json::to_string(&NamespaceSelection::All).unwrap(),
            r#""all""#
        );
        assert_eq!(
            serde_json::to_string(&set(&["b", "a"])).unwrap(),
            r#"{"set":["a","b"]}"#
        );
        // Hand-edited settings cannot smuggle in an empty set.
        let sel: NamespaceSelection = serde_json::from_str(r#"{"set":[]}"#).unwrap();
        assert_eq!(sel, NamespaceSelection::All);
        let sel: NamespaceSelection = serde_json::from_str(r#"{"set":["x","x",""]}"#).unwrap();
        assert_eq!(sel, set(&["x"]));
    }

    // --- Favourites ---

    #[test]
    fn favourites_add_remove_toggle_keep_order() {
        let mut fav = NamespaceFavourites::new();
        assert!(fav.is_empty());
        assert!(fav.add("prod"));
        assert!(fav.add("dev"));
        assert!(!fav.add("prod"));
        assert!(!fav.add(" "));
        assert_eq!(fav.iter().collect::<Vec<_>>(), ["prod", "dev"]);
        assert!(fav.add("qa"));
        assert!(fav.remove("dev"));
        assert!(!fav.remove("dev"));
        assert_eq!(fav.iter().collect::<Vec<_>>(), ["prod", "qa"]);
        assert!(fav.contains("qa"));
        assert!(!fav.toggle("qa"));
        assert!(fav.toggle("dev"));
        assert_eq!(fav.iter().collect::<Vec<_>>(), ["prod", "dev"]);
        assert_eq!(fav.len(), 2);
    }

    #[test]
    fn favourites_serde_round_trips_as_a_list() {
        let fav: NamespaceFavourites = ["b", "a", "b", ""].into_iter().collect();
        let json = serde_json::to_string(&fav).unwrap();
        assert_eq!(json, r#"["b","a"]"#);
        assert_eq!(
            serde_json::from_str::<NamespaceFavourites>(&json).unwrap(),
            fav
        );
    }

    // --- WatchScope ---

    #[test]
    fn all_with_namespaced_kind_is_cluster_wide() {
        let scope = WatchScope::derive(&NamespaceSelection::All, Scope::Namespaced);
        assert_eq!(scope, WatchScope::Cluster);
        assert_eq!(scope.watch_count(), 1);
        assert!(scope.namespaces().is_empty());
    }

    #[test]
    fn two_namespaces_give_two_watches() {
        let scope = WatchScope::derive(&set(&["prod", "dev"]), Scope::Namespaced);
        assert_eq!(
            scope,
            WatchScope::Namespaces(vec!["dev".into(), "prod".into()])
        );
        assert_eq!(scope.watch_count(), 2);
        assert_eq!(scope.namespaces(), ["dev", "prod"]);
    }

    #[test]
    fn cluster_scoped_kind_ignores_the_selection() {
        for sel in [NamespaceSelection::All, set(&["a", "b"])] {
            assert_eq!(
                WatchScope::derive(&sel, Scope::Cluster),
                WatchScope::Cluster
            );
        }
    }

    #[test]
    fn watch_scope_serde_round_trips() {
        for scope in [
            WatchScope::Cluster,
            WatchScope::Namespaces(vec!["a".into(), "b".into()]),
        ] {
            let json = serde_json::to_string(&scope).unwrap();
            assert_eq!(serde_json::from_str::<WatchScope>(&json).unwrap(), scope);
        }
    }

    proptest! {
        #[test]
        fn selection_is_never_an_empty_set(names in proptest::collection::vec("[ a-z-]{0,6}", 0..8)) {
            let sel = NamespaceSelection::from_names(&names);
            if let NamespaceSelection::Set(set) = &sel {
                prop_assert!(!set.is_empty());
                prop_assert!(set.iter().all(|n| !n.is_empty() && n == n.trim()));
            }
            let json = serde_json::to_string(&sel).unwrap();
            prop_assert_eq!(serde_json::from_str::<NamespaceSelection>(&json).unwrap(), sel);
        }

        #[test]
        fn watch_count_matches_selection(names in proptest::collection::vec("[a-z]{1,6}", 0..8)) {
            let sel = NamespaceSelection::from_names(&names);
            let scope = WatchScope::derive(&sel, Scope::Namespaced);
            let expected = sel.names().count().max(1);
            prop_assert_eq!(scope.watch_count(), expected);
            prop_assert_eq!(WatchScope::derive(&sel, Scope::Cluster), WatchScope::Cluster);
        }

        #[test]
        fn random_event_sequences_stay_consistent(kinds in proptest::collection::vec(0usize..7, 0..40)) {
            let mut state = ClusterSessionState::default();
            for i in kinds {
                let kind = SessionEventKind::ALL[i];
                let before = state.phase();
                match state.clone().transition(event_for(kind)) {
                    Ok(next) => {
                        prop_assert_eq!(Some(next.phase()), before.next(kind));
                        state = next;
                    }
                    Err(e) => {
                        prop_assert_eq!(before.next(kind), None);
                        prop_assert_eq!(e.from, before);
                    }
                }
            }
        }
    }
}
