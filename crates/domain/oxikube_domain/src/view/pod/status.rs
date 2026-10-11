//! The `READY`, `STATUS` and `RESTARTS` columns of `kubectl get pods`, derived from the pod's JSON
//! by the rules of kubectl's pod printer (`printPod`, see the [module docs](super)).

use std::borrow::Cow;

use jiff::Timestamp;

use crate::json::{Array, JsonRef};
use crate::view::{arr_of, bool_of, count_of, i32_of, obj_of, str_of, sub, ts_of};

use super::PodPhase;

/// The printer's derived columns, before conversion to owned output.
pub(super) struct StatusRow<'a> {
    pub(super) reason: Cow<'a, str>,
    pub(super) ready: u32,
    pub(super) total: u32,
    pub(super) restarts: u32,
    pub(super) last_restart: Option<Timestamp>,
}

impl<'a> StatusRow<'a> {
    pub(super) fn compute(spec: JsonRef<'a>, status: JsonRef<'a>, deleting: bool) -> Self {
        let phase = str_of(status, "phase");
        let status_reason = str_of(status, "reason");
        let mut reason = Cow::Borrowed(status_reason.or(phase).unwrap_or("Unknown"));

        let conditions = arr_of(status, "conditions");
        if conditions.iter().any(|c| {
            str_of(c, "type") == Some("PodScheduled")
                && str_of(c, "reason") == Some("SchedulingGated")
        }) {
            reason = Cow::Borrowed("SchedulingGated");
        }

        let init_specs = arr_of(spec, "initContainers");
        let sidecars = init_specs.iter().filter(|&c| is_restartable(c)).count();
        let total = len_u32(arr_of(spec, "containers").len() + sidecars);

        let mut ready = 0u32;
        let mut restarts = 0u32;
        let mut last_restart = None;
        let mut sidecar_restarts = 0u32;
        let mut sidecar_last_restart = None;
        let mut initializing = false;

        for (i, cs) in arr_of(status, "initContainerStatuses").iter().enumerate() {
            let count = count_of(cs, "restartCount");
            let finished = last_finished_at(cs);
            restarts = restarts.saturating_add(count);
            last_restart = later(last_restart, finished);
            let restartable = str_of(cs, "name")
                .and_then(|name| init_specs.iter().find(|&c| str_of(c, "name") == Some(name)))
                .is_some_and(is_restartable);
            if restartable {
                sidecar_restarts = sidecar_restarts.saturating_add(count);
                sidecar_last_restart = later(sidecar_last_restart, finished);
            }

            let state = StateView::of(cs);
            if state.terminated.is_some_and(|t| i32_of(t, "exitCode") == 0) {
                continue;
            }
            if restartable && bool_of(cs, "started") {
                if bool_of(cs, "ready") {
                    ready += 1;
                }
                continue;
            }
            initializing = true;
            let waiting_reason = state
                .waiting
                .and_then(|w| str_of(w, "reason"))
                .filter(|r| *r != "PodInitializing");
            reason = Cow::Owned(match (state.terminated, waiting_reason) {
                (Some(t), _) => match str_of(t, "reason") {
                    Some(r) => format!("Init:{r}"),
                    None => exit_text("Init:", t),
                },
                (None, Some(r)) => format!("Init:{r}"),
                (None, None) => format!("Init:{i}/{}", init_specs.len()),
            });
            break;
        }

        if !initializing || initialized(conditions) {
            restarts = sidecar_restarts;
            last_restart = sidecar_last_restart;
            let mut has_running = false;
            let mut error_reason = None;
            for cs in arr_of(status, "containerStatuses").iter_rev() {
                restarts = restarts.saturating_add(count_of(cs, "restartCount"));
                last_restart = later(last_restart, last_finished_at(cs));
                let state = StateView::of(cs);
                if let Some(r) = state.waiting.and_then(|w| str_of(w, "reason")) {
                    reason = Cow::Borrowed(r);
                } else if let Some(t) = state.terminated {
                    reason = match str_of(t, "reason") {
                        Some(r) => Cow::Borrowed(r),
                        None => Cow::Owned(exit_text("", t)),
                    };
                    if i32_of(t, "exitCode") != 0 {
                        error_reason = Some(reason.clone());
                    }
                } else if bool_of(cs, "ready") && state.running.is_some() {
                    has_running = true;
                    ready += 1;
                }
            }
            if reason == "Completed" {
                let pod_ready = conditions.iter().any(|c| {
                    str_of(c, "type") == Some("Ready") && str_of(c, "status") == Some("True")
                });
                if has_running && pod_ready {
                    reason = Cow::Borrowed("Running");
                } else if let Some(err) = error_reason {
                    reason = err;
                } else if has_running {
                    reason = Cow::Borrowed("NotReady");
                }
            }
        }

        if deleting && status_reason == Some("NodeLost") {
            reason = Cow::Borrowed("Unknown");
        } else if deleting && !PodPhase::parse(phase).is_terminal() {
            reason = Cow::Borrowed("Terminating");
        }

        Self {
            reason,
            ready,
            total,
            restarts,
            last_restart,
        }
    }
}

/// The first `Initialized` condition is `True`.
fn initialized(conditions: Array<'_>) -> bool {
    conditions
        .iter()
        .find(|&c| str_of(c, "type") == Some("Initialized"))
        .is_some_and(|c| str_of(c, "status") == Some("True"))
}

/// An init container spec with `restartPolicy: Always` (a sidecar).
pub(in crate::view) fn is_restartable(spec: JsonRef<'_>) -> bool {
    str_of(spec, "restartPolicy") == Some("Always")
}

/// `Signal:N` or `ExitCode:N` (with `prefix`) for a terminated state with no reason.
fn exit_text(prefix: &str, terminated: JsonRef<'_>) -> String {
    match i32_of(terminated, "signal") {
        0 => format!("{prefix}ExitCode:{}", i32_of(terminated, "exitCode")),
        sig => format!("{prefix}Signal:{sig}"),
    }
}

/// `lastState.terminated.finishedAt` of a container status.
fn last_finished_at(cs: JsonRef<'_>) -> Option<Timestamp> {
    obj_of(sub(cs, "lastState"), "terminated").and_then(|t| ts_of(t, "finishedAt"))
}

fn later(a: Option<Timestamp>, b: Option<Timestamp>) -> Option<Timestamp> {
    a.max(b)
}

fn len_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Borrowed view of a container status's `state` (each key present only when set).
pub(in crate::view) struct StateView<'a> {
    pub(in crate::view) waiting: Option<JsonRef<'a>>,
    pub(in crate::view) running: Option<JsonRef<'a>>,
    pub(in crate::view) terminated: Option<JsonRef<'a>>,
}

impl<'a> StateView<'a> {
    pub(in crate::view) fn of(container_status: JsonRef<'a>) -> Self {
        let state = sub(container_status, "state");
        Self {
            waiting: obj_of(state, "waiting"),
            running: obj_of(state, "running"),
            terminated: obj_of(state, "terminated"),
        }
    }
}
