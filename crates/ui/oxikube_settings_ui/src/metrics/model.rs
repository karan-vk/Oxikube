//! [`MetricsSettingsModel`]: what the cluster's metrics page shows, as plain data.
//!
//! The view draws this and nothing else, so every state (found, not found, forbidden, failed,
//! disabled, not connected, running) is a value a unit test builds without a window. It owns the
//! wording: `metrics unavailable: <reason>` for absence, one tone per row.

use jiff::tz::TimeZone;
use oxikube_app::{ProviderState, ProviderStatus};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{
    DetectionFailure, DetectionState, MetricsMode, Probe, ProbeOutcome, PrometheusAuthKind,
    PrometheusOverride,
};

/// How a row reads: which status colour it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Plain text.
    Neutral,
    /// Fine: a provider answered.
    Good,
    /// In progress.
    Pending,
    /// Needs attention but is not broken: not checked, no usable port.
    Warning,
    /// Broken.
    Error,
    /// Nothing to say: a service that is simply not there.
    Muted,
}

/// The headline: one line saying where the cluster's history metrics stand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Headline {
    /// The text (`Using kube-prometheus at monitoring/prometheus-operated:web`).
    pub text: String,
    /// Its tone.
    pub tone: Tone,
}

/// One service detection looked at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeLine {
    /// `namespace/service` (a name ending in `*` is a prefix).
    pub service: String,
    /// The stack it would belong to.
    pub stack: String,
    /// What happened.
    pub outcome: String,
    /// Its tone.
    pub tone: Tone,
}

/// One setting in effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingLine {
    /// The setting (`Provider`).
    pub label: &'static str,
    /// Its value, as the user would write it.
    pub value: String,
}

/// Everything the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsSettingsModel {
    /// The cluster's name.
    pub cluster_title: String,
    /// The headline.
    pub headline: Headline,
    /// `metrics unavailable: <reason>`, when there is no provider and nothing is running.
    pub unavailable: Option<String>,
    /// Whether the cluster's history metrics are switched on (`prometheus.enabled`).
    pub enabled: bool,
    /// Whether a run is in progress (the "look again" button waits).
    pub busy: bool,
    /// Whether "look again" can do anything: connected, on, and not already running.
    pub can_redetect: bool,
    /// The services detection looked at, in catalogue order.
    pub probes: Vec<ProbeLine>,
    /// What to tell the user about the probe list (`Looked at 14 services`, or why it is empty).
    pub probes_note: String,
    /// The settings in effect.
    pub settings: Vec<SettingLine>,
    /// Where to change them.
    pub edit_hint: String,
    /// When detection last finished, as a clock time.
    pub checked: Option<String>,
}

impl MetricsSettingsModel {
    /// The page for `cluster` (shown as `title`) in `status`; `connected` says whether the
    /// cluster's session is up.
    pub fn build(
        cluster: &ClusterId,
        title: &str,
        connected: bool,
        status: &ProviderStatus,
    ) -> Self {
        let settings = &status.settings;
        let enabled = settings.enabled;
        let busy = status.is_detecting();
        Self {
            cluster_title: title.to_owned(),
            headline: headline(status),
            unavailable: status.unavailable_text(),
            enabled,
            busy,
            can_redetect: connected && enabled && settings.mode != MetricsMode::Off && !busy,
            probes: status.probes.iter().map(probe_line).collect(),
            probes_note: probes_note(status),
            settings: setting_lines(settings),
            edit_hint: format!(
                "Change these in settings.json under clusters.{cluster}.prometheus."
            ),
            checked: status.checked_at.map(clock_time),
        }
    }
}

fn headline(status: &ProviderStatus) -> Headline {
    let (text, tone) = match &status.state {
        ProviderState::Disabled => ("History metrics are off".to_owned(), Tone::Muted),
        ProviderState::WaitingForCluster => {
            ("Waiting for the cluster to connect".to_owned(), Tone::Muted)
        }
        ProviderState::Detection(DetectionState::Detecting) => (status.headline(), Tone::Pending),
        ProviderState::Detection(DetectionState::Found(_)) => (status.headline(), Tone::Good),
        ProviderState::Detection(DetectionState::NotFound) => (
            "No Prometheus-compatible service found".to_owned(),
            Tone::Warning,
        ),
        ProviderState::Detection(DetectionState::Failed(failure)) => (
            match (failure, status.mode) {
                (DetectionFailure::Forbidden(_), _) => "Not allowed to look for a metrics service",
                (DetectionFailure::ProbeFailed { .. }, MetricsMode::Manual) => {
                    "The configured endpoint did not answer"
                }
                (DetectionFailure::ProbeFailed { .. }, _) => "A metrics service did not answer",
                (DetectionFailure::Other(_), MetricsMode::Manual) => {
                    "The configured endpoint cannot be used"
                }
                (DetectionFailure::Other(_), _) => "Could not look for a metrics service",
            }
            .to_owned(),
            Tone::Error,
        ),
    };
    Headline { text, tone }
}

fn probe_line(probe: &Probe) -> ProbeLine {
    let tone = match probe.outcome {
        ProbeOutcome::Answered => Tone::Good,
        ProbeOutcome::NotPresent | ProbeOutcome::NotNeeded => Tone::Muted,
        ProbeOutcome::NoUsablePort | ProbeOutcome::Skipped(_) => Tone::Warning,
        ProbeOutcome::Failed(_) => Tone::Error,
    };
    ProbeLine {
        service: probe.label(),
        stack: probe.kind.label().to_owned(),
        outcome: probe.outcome.text(),
        tone,
    }
}

fn probes_note(status: &ProviderStatus) -> String {
    match (&status.state, status.probes.len(), status.mode) {
        (_, 0, MetricsMode::Manual) => {
            "A configured endpoint is checked as it is; nothing is searched for.".to_owned()
        }
        (_, 0, MetricsMode::Off) => "Nothing is looked for while metrics are off.".to_owned(),
        (ProviderState::Detection(DetectionState::Detecting), 0, _) => {
            "Listing the services of the namespaces monitoring stacks use…".to_owned()
        }
        (_, 0, _) => "Nothing has been looked at yet.".to_owned(),
        (_, n, _) => format!("Looked at {n} known services."),
    }
}

fn setting_lines(settings: &PrometheusOverride) -> Vec<SettingLine> {
    let line = |label, value: String| SettingLine { label, value };
    let mut lines = vec![
        line("Enabled", yes_no(settings.enabled).to_owned()),
        line("Provider", settings.mode.id().to_owned()),
    ];
    if settings.mode != MetricsMode::Manual {
        return lines;
    }
    if let Some(kind) = settings.kind {
        lines.push(line("Stack", kind.label().to_owned()));
    }
    match (&settings.url, &settings.path) {
        (Some(url), _) => lines.push(line("URL", url.clone())),
        (None, Some(path)) => lines.push(line("Service", path.clone())),
        (None, None) => lines.push(line("Endpoint", "not set".to_owned())),
    }
    if let Some(prefix) = &settings.path_prefix {
        lines.push(line("Path prefix", prefix.clone()));
    }
    let auth = match settings.auth {
        PrometheusAuthKind::None => "none".to_owned(),
        PrometheusAuthKind::Bearer => {
            format!("bearer token in keychain entry {}", secret_name(settings))
        }
        PrometheusAuthKind::Basic => format!(
            "basic, user {}, password in keychain entry {}",
            settings.auth_user.as_deref().unwrap_or("(not set)"),
            secret_name(settings)
        ),
    };
    lines.push(line("Authentication", auth));
    lines
}

/// The keychain entry's name (never its value: nothing here can read it).
fn secret_name(settings: &PrometheusOverride) -> String {
    settings
        .auth_secret
        .as_ref()
        .map_or_else(|| "(not set)".to_owned(), ToString::to_string)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// `HH:MM:SS` in the system time zone.
fn clock_time(at: jiff::Timestamp) -> String {
    at.to_zoned(TimeZone::system())
        .strftime("%H:%M:%S")
        .to_string()
}
