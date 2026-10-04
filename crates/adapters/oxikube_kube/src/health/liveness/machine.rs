//! The decision core of the liveness loop: probe results in, health events out. No I/O,
//! no time, so the policy is tested exhaustively without a runtime.

use oxikube_domain::session::SessionEvent;
use oxikube_domain::{ErrorKind, OxiError};

/// A fact about the connection, for the session manager to map onto the session state
/// machine (see [`HealthEvent::to_session_event`]).
#[derive(Debug)]
pub enum HealthEvent {
    /// The probe succeeded.
    Healthy {
        /// The apiserver's `gitVersion`.
        server_version: String,
    },
    /// A probe failed. The session should go (or stay) `Degraded`.
    Unhealthy {
        /// Why the probe failed.
        error: OxiError,
        /// How many probes in a row have failed, including this one.
        consecutive_failures: u32,
    },
    /// The connection is not coming back by itself: a permanent failure (non-retryable
    /// `Auth`, or a TLS/certificate error), or the failure threshold was reached. The session should go to `Error`. The probe loop
    /// stops after emitting this; restart it after the session reconnects.
    Failed {
        /// The error that ended the probing.
        error: OxiError,
    },
}

impl HealthEvent {
    /// The error carried by `Unhealthy` and `Failed`.
    pub fn error(&self) -> Option<&OxiError> {
        match self {
            HealthEvent::Healthy { .. } => None,
            HealthEvent::Unhealthy { error, .. } | HealthEvent::Failed { error } => Some(error),
        }
    }

    /// The event to feed to `ClusterSessionState::transition`.
    ///
    /// `Failed` becomes [`SessionEvent::Failed`] with the (already redacted) error text.
    /// A session manager that wants to route a non-retryable `Auth` failure to
    /// `AuthRequired` can inspect [`error`](Self::error) first.
    pub fn to_session_event(&self) -> SessionEvent {
        match self {
            HealthEvent::Healthy { .. } => SessionEvent::Healthy,
            HealthEvent::Unhealthy { .. } => SessionEvent::Unhealthy,
            HealthEvent::Failed { error } => SessionEvent::Failed {
                reason: error.to_string(),
            },
        }
    }
}

/// The failure policy.
///
/// * Every success emits `Healthy` and clears the failure count.
/// * The first failure of a run always emits `Unhealthy` (the cluster is `Degraded`
///   before it is anything worse).
/// * A failure ends the run with `Failed` when it is the `threshold`-th in a row, or at
///   once when it is *permanent*: a non-retryable `Auth` error (revoked or missing
///   credential) or a non-retryable `Network` error (a TLS or certificate failure). A
///   permanent first failure therefore emits `Unhealthy` then `Failed` together.
/// * Other non-retryable kinds (`Forbidden` on `/version` on a hardened cluster,
///   `NotFound` or `Internal` from a gateway that does not proxy `/version`) are not
///   proof the cluster is unusable, so they count toward the threshold like any other
///   failure.
#[derive(Debug)]
pub(super) struct HealthMachine {
    threshold: u32,
    failures: u32,
    done: bool,
}

impl HealthMachine {
    /// `threshold` consecutive failures end the run (at least 1).
    pub(super) fn new(threshold: u32) -> Self {
        Self {
            threshold: threshold.max(1),
            failures: 0,
            done: false,
        }
    }

    /// True once `Failed` has been emitted.
    pub(super) fn is_done(&self) -> bool {
        self.done
    }

    pub(super) fn on_probe(&mut self, result: Result<String, OxiError>) -> Vec<HealthEvent> {
        match result {
            Ok(server_version) => {
                self.failures = 0;
                vec![HealthEvent::Healthy { server_version }]
            }
            Err(error) => {
                self.failures += 1;
                let n = self.failures;
                let terminal = is_permanent(&error) || n >= self.threshold;
                let mut events = Vec::with_capacity(2);
                if n == 1 || !terminal {
                    events.push(HealthEvent::Unhealthy {
                        error: copy_of(&error),
                        consecutive_failures: n,
                    });
                }
                if terminal {
                    self.done = true;
                    events.push(HealthEvent::Failed { error });
                }
                events
            }
        }
    }
}

/// A failure retrying cannot fix.
fn is_permanent(error: &OxiError) -> bool {
    !error.is_retryable() && matches!(error.kind(), ErrorKind::Auth | ErrorKind::Network)
}

/// `OxiError` is not `Clone`; an event pair needs the error twice. The source (if any) is
/// dropped from the copy: it is already part of the text.
fn copy_of(e: &OxiError) -> OxiError {
    OxiError::new(e.kind(), e.message()).with_retryable(e.is_retryable())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transient() -> Result<String, OxiError> {
        Err(OxiError::auth("token expired", true))
    }

    fn kinds(events: &[HealthEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                HealthEvent::Healthy { .. } => "healthy",
                HealthEvent::Unhealthy { .. } => "unhealthy",
                HealthEvent::Failed { .. } => "failed",
            })
            .collect()
    }

    #[test]
    fn success_is_healthy_every_time() {
        let mut m = HealthMachine::new(3);
        assert_eq!(kinds(&m.on_probe(Ok("v1.35".into()))), ["healthy"]);
        assert_eq!(kinds(&m.on_probe(Ok("v1.35".into()))), ["healthy"]);
    }

    #[test]
    fn retryable_failures_degrade_then_fail_at_the_threshold() {
        let mut m = HealthMachine::new(3);
        assert_eq!(kinds(&m.on_probe(transient())), ["unhealthy"]);
        assert_eq!(kinds(&m.on_probe(transient())), ["unhealthy"]);
        assert!(!m.is_done());
        assert_eq!(kinds(&m.on_probe(transient())), ["failed"]);
        assert!(m.is_done());
    }

    #[test]
    fn failure_counts_are_reported() {
        let mut m = HealthMachine::new(5);
        m.on_probe(transient());
        let ev = m.on_probe(transient());
        assert!(matches!(
            ev[0],
            HealthEvent::Unhealthy {
                consecutive_failures: 2,
                ..
            }
        ));
    }

    #[test]
    fn recovery_resets_the_count() {
        let mut m = HealthMachine::new(3);
        m.on_probe(transient());
        m.on_probe(transient());
        assert_eq!(kinds(&m.on_probe(Ok("v".into()))), ["healthy"]);
        // Two more failures are again below the threshold.
        assert_eq!(kinds(&m.on_probe(transient())), ["unhealthy"]);
        assert_eq!(kinds(&m.on_probe(transient())), ["unhealthy"]);
        assert!(!m.is_done());
    }

    #[test]
    fn non_retryable_first_failure_is_degraded_then_error_in_one_go() {
        let mut m = HealthMachine::new(3);
        let ev = m.on_probe(Err(OxiError::auth("revoked", false)));
        assert_eq!(kinds(&ev), ["unhealthy", "failed"]);
        assert!(m.is_done());
    }

    #[test]
    fn permanent_later_failure_is_just_failed() {
        let mut m = HealthMachine::new(5);
        m.on_probe(transient());
        let ev = m.on_probe(Err(OxiError::auth("revoked", false)));
        assert_eq!(kinds(&ev), ["failed"]);
    }

    #[test]
    fn permanent_tls_failure_fails_at_once() {
        let mut m = HealthMachine::new(3);
        let tls = OxiError::network("certificate has expired").with_retryable(false);
        assert_eq!(kinds(&m.on_probe(Err(tls))), ["unhealthy", "failed"]);
    }

    #[test]
    fn other_non_retryable_kinds_count_toward_the_threshold() {
        let makers: [fn() -> OxiError; 4] = [
            || OxiError::forbidden("/version is forbidden"),
            || OxiError::not_found("gateway has no /version"),
            || OxiError::internal("bad gateway body"),
            || OxiError::unsupported("no such endpoint"),
        ];
        for make in makers {
            let mut m = HealthMachine::new(3);
            assert_eq!(kinds(&m.on_probe(Err(make()))), ["unhealthy"]);
            assert_eq!(kinds(&m.on_probe(Err(make()))), ["unhealthy"]);
            assert!(!m.is_done());
            assert_eq!(kinds(&m.on_probe(Err(make()))), ["failed"]);
        }
    }

    #[test]
    fn a_forbidden_version_probe_recovers_if_it_starts_working() {
        let mut m = HealthMachine::new(3);
        m.on_probe(Err(OxiError::forbidden("no")));
        m.on_probe(Err(OxiError::forbidden("no")));
        assert_eq!(kinds(&m.on_probe(Ok("v".into()))), ["healthy"]);
        assert!(!m.is_done());
    }

    #[test]
    fn network_errors_are_retryable_by_default() {
        let mut m = HealthMachine::new(2);
        assert_eq!(
            kinds(&m.on_probe(Err(OxiError::network("reset")))),
            ["unhealthy"]
        );
        assert_eq!(
            kinds(&m.on_probe(Err(OxiError::timeout("slow")))),
            ["failed"]
        );
    }

    #[test]
    fn threshold_of_one_fails_on_the_first_failure() {
        let mut m = HealthMachine::new(1);
        assert_eq!(kinds(&m.on_probe(transient())), ["unhealthy", "failed"]);
        assert_eq!(HealthMachine::new(0).threshold, 1);
    }

    #[test]
    fn the_copy_keeps_kind_message_and_flag() {
        let mut m = HealthMachine::new(1);
        let ev = m.on_probe(Err(OxiError::auth("revoked", false)));
        for e in &ev {
            let err = e.error().unwrap();
            assert_eq!(err.kind(), ErrorKind::Auth);
            assert_eq!(err.message(), "revoked");
            assert!(!err.is_retryable());
        }
    }

    #[test]
    fn events_map_onto_session_events() {
        use oxikube_domain::session::SessionEventKind as K;
        let healthy = HealthEvent::Healthy {
            server_version: "v".into(),
        };
        assert_eq!(healthy.to_session_event().kind(), K::Healthy);
        let mut m = HealthMachine::new(1);
        let ev = m.on_probe(transient());
        assert_eq!(ev[0].to_session_event().kind(), K::Unhealthy);
        match ev[1].to_session_event() {
            SessionEvent::Failed { reason } => assert!(reason.contains("token expired")),
            other => panic!("{other:?}"),
        }
    }
}
