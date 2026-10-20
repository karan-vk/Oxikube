//! The text of the cells: bytes, traffic and the status of a forward.

use std::time::Duration;

use oxikube_app::{ForwardInfo, ForwardState};
use oxikube_domain::Age;
use oxikube_ports::ForwardStatsSnapshot;

/// `1536` as `1.5 KB`, `12` as `12 B`: binary units, one decimal from a kilobyte up.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `↓ 1.2 MB ↑ 34 B · 2 conn`: bytes from and to the pod, and the open connections.
pub fn traffic(stats: &ForwardStatsSnapshot) -> String {
    format!(
        "\u{2193} {} \u{2191} {} \u{b7} {} conn",
        bytes(stats.bytes_in),
        bytes(stats.bytes_out),
        stats.active_connections
    )
}

/// The status column: the state, and for a reconnect the attempt and when the next one is.
pub fn status_text(info: &ForwardInfo) -> String {
    match &info.state {
        ForwardState::Starting => "Starting".to_owned(),
        ForwardState::Active { pod, .. } => format!("Active \u{b7} {pod}"),
        ForwardState::Reconnecting { attempt, delay, .. } => {
            if delay.is_zero() {
                format!("Reconnecting \u{b7} attempt {attempt}, retrying now")
            } else {
                format!(
                    "Reconnecting \u{b7} attempt {attempt}, next in {}",
                    delay_text(*delay)
                )
            }
        }
        ForwardState::Failed(failure) => format!("Failed \u{b7} {}", failure.message),
        ForwardState::Stopped => "Stopped".to_owned(),
    }
}

/// `3s`, `1m 30s`: a retry delay, rounded up to whole seconds.
fn delay_text(delay: Duration) -> String {
    let secs = delay.as_secs() + u64::from(delay.subsec_nanos() > 0);
    match (secs / 60, secs % 60) {
        (0, s) => format!("{s}s"),
        (m, 0) => format!("{m}m"),
        (m, s) => format!("{m}m {s}s"),
    }
}

/// The uptime of a running forward (`5m`, `2h`), `-` for one that is not running.
pub fn uptime(info: &ForwardInfo, now: jiff::Timestamp) -> String {
    if info.state.is_running() {
        Age::between(info.started_at, now).to_string()
    } else {
        "-".to_owned()
    }
}

/// The ports column: `local -> remote`, the local port once it is bound.
pub fn ports(info: &ForwardInfo) -> String {
    let local = info
        .local_addr
        .map_or(info.spec.local_port, |addr| addr.port());
    let local = if local == 0 {
        "auto".to_owned()
    } else {
        local.to_string()
    };
    format!("{local} -> {}", info.spec.remote_port)
}

/// The target column: `namespace/kind/name`.
pub fn target(info: &ForwardInfo) -> String {
    let target = &info.spec.target;
    let kind = target.gvk.kind.to_ascii_lowercase();
    match target.namespace.as_deref() {
        Some(ns) => format!("{ns}/{kind}/{}", target.name),
        None => format!("{kind}/{}", target.name),
    }
}

/// The line under the table for a forward that needs words: why it failed with what to do about
/// it, or the last connection error.
pub fn detail(info: &ForwardInfo) -> Option<String> {
    match &info.state {
        ForwardState::Failed(failure) => Some(match failure.suggestion() {
            Some(hint) => format!("{}. {hint}", failure.message),
            None => failure.message.clone(),
        }),
        ForwardState::Reconnecting { reason, .. } if !reason.is_empty() => Some(reason.clone()),
        _ => info.last_error.clone(),
    }
}
