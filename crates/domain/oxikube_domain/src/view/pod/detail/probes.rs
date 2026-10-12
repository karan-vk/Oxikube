//! The probes of a container, spelled as `kubectl describe` does.

use crate::json::JsonRef;
use crate::view::{i64_of, obj_of, str_of};

/// Which probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// `livenessProbe`.
    Liveness,
    /// `readinessProbe`.
    Readiness,
    /// `startupProbe`.
    Startup,
}

impl ProbeKind {
    /// `liveness`, `readiness` or `startup`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Liveness => "liveness",
            Self::Readiness => "readiness",
            Self::Startup => "startup",
        }
    }
}

/// One probe of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeDetail {
    /// Which probe.
    pub kind: ProbeKind,
    /// What it does: `http-get http://:8080/healthz`, `tcp-socket :3306`, `exec [cat /tmp/ok]`,
    /// `grpc :9000`.
    pub action: String,
    /// `delay=10s timeout=1s period=10s #success=1 #failure=3`.
    pub timing: String,
}

pub(super) fn probes_of(container: JsonRef<'_>) -> Vec<ProbeDetail> {
    [
        ("livenessProbe", ProbeKind::Liveness),
        ("readinessProbe", ProbeKind::Readiness),
        ("startupProbe", ProbeKind::Startup),
    ]
    .into_iter()
    .filter_map(|(key, kind)| {
        let probe = obj_of(container, key)?;
        Some(ProbeDetail {
            kind,
            action: action(probe),
            timing: timing(probe),
        })
    })
    .collect()
}

fn action(probe: JsonRef<'_>) -> String {
    if let Some(http) = obj_of(probe, "httpGet") {
        let scheme = str_of(http, "scheme").unwrap_or("HTTP").to_lowercase();
        let host = str_of(http, "host").unwrap_or("");
        let path = str_of(http, "path").unwrap_or("/");
        return format!("http-get {scheme}://{host}:{}{path}", port(http));
    }
    if let Some(tcp) = obj_of(probe, "tcpSocket") {
        return format!(
            "tcp-socket {}:{}",
            str_of(tcp, "host").unwrap_or(""),
            port(tcp)
        );
    }
    if let Some(grpc) = obj_of(probe, "grpc") {
        return format!("grpc :{}", port(grpc));
    }
    if let Some(exec) = obj_of(probe, "exec") {
        let command: Vec<&str> = crate::view::arr_of(exec, "command")
            .iter()
            .filter_map(JsonRef::as_str)
            .collect();
        return format!("exec [{}]", command.join(" "));
    }
    "unknown".to_owned()
}

/// A probe's port: a number or a name.
fn port(action: JsonRef<'_>) -> String {
    match action.get("port") {
        Some(port) => match (port.as_i64(), port.as_str()) {
            (Some(number), _) => number.to_string(),
            (None, Some(name)) => name.to_owned(),
            _ => String::new(),
        },
        None => String::new(),
    }
}

/// The probe's timing with the API defaults (`timeoutSeconds` 1, `periodSeconds` 10, success 1,
/// failure 3) filled in, as `kubectl describe` prints it.
fn timing(probe: JsonRef<'_>) -> String {
    let or = |key: &str, default: i64| i64_of(probe, key).unwrap_or(default);
    format!(
        "delay={}s timeout={}s period={}s #success={} #failure={}",
        or("initialDelaySeconds", 0),
        or("timeoutSeconds", 1),
        or("periodSeconds", 10),
        or("successThreshold", 1),
        or("failureThreshold", 3),
    )
}
