//! The page as data: every state's wording and tone, without a window.

use oxikube_app::{ProviderState, ProviderStatus};
use oxikube_ports::{DetectionState, MetricsMode};

use super::super::MetricsSettingsModel;
use super::super::Tone;
use super::super::test_support::{
    cluster, detecting, disabled, forbidden, found, manual, not_found, probe_failed,
};

fn model(status: &ProviderStatus, connected: bool) -> MetricsSettingsModel {
    MetricsSettingsModel::build(&cluster(), "kind-oxikube", connected, status)
}

#[test]
fn a_found_provider_is_a_good_headline_with_no_unavailable_text() {
    let m = model(&found(), true);
    assert_eq!(m.headline.tone, Tone::Good);
    assert_eq!(
        m.headline.text,
        "Using kube-prometheus at monitoring/prometheus-operated:web"
    );
    assert_eq!(m.unavailable, None);
    assert!(m.can_redetect);
    assert!(!m.busy);
}

#[test]
fn not_found_says_metrics_unavailable_and_lists_what_was_looked_at() {
    let m = model(&not_found(), true);
    assert_eq!(m.headline.tone, Tone::Warning);
    assert_eq!(m.headline.text, "No Prometheus-compatible service found");
    let reason = m.unavailable.unwrap();
    assert!(reason.starts_with("metrics unavailable: "), "{reason}");
    assert_eq!(m.probes.len(), 3);
    assert_eq!(m.probes[0].service, "monitoring/prometheus-operated");
    assert_eq!(m.probes[0].stack, "kube-prometheus");
    assert_eq!(m.probes[0].outcome, "not found");
    assert_eq!(m.probes[0].tone, Tone::Muted);
    assert_eq!(m.probes_note, "Looked at 3 known services.");
}

#[test]
fn forbidden_is_an_error_that_says_to_configure_by_hand() {
    let m = model(&forbidden(), true);
    assert_eq!(m.headline.tone, Tone::Error);
    assert_eq!(m.headline.text, "Not allowed to look for a metrics service");
    assert!(
        m.unavailable
            .unwrap()
            .contains("configure the endpoint by hand")
    );
    assert_eq!(m.probes[0].tone, Tone::Warning);
    assert!(m.probes[0].outcome.starts_with("not checked: "));
}

#[test]
fn a_matched_service_that_did_not_answer_names_it_and_why() {
    let m = model(&probe_failed(), true);
    assert_eq!(m.headline.tone, Tone::Error);
    assert_eq!(m.headline.text, "A metrics service did not answer");
    let reason = m.unavailable.unwrap();
    assert!(
        reason.contains("monitoring/prometheus-operated:web"),
        "{reason}"
    );
    assert!(reason.contains("no ready endpoint"), "{reason}");
    assert_eq!(m.probes[0].tone, Tone::Error);
}

#[test]
fn a_running_detection_is_pending_and_waits_for_its_own_end() {
    let m = model(&detecting(), true);
    assert_eq!(m.headline.tone, Tone::Pending);
    assert!(m.busy);
    assert!(!m.can_redetect, "a run in progress is not started twice");
    assert_eq!(m.unavailable, None, "it is not unavailable yet");
}

#[test]
fn switched_off_is_muted_and_can_be_switched_on_again() {
    let m = model(&disabled(), true);
    assert_eq!(m.headline.tone, Tone::Muted);
    assert!(!m.enabled);
    assert!(!m.can_redetect);
    assert!(m.unavailable.unwrap().contains("turned off"));
    assert_eq!(
        m.probes_note,
        "Nothing is looked for while metrics are off."
    );
}

#[test]
fn a_cluster_that_is_not_connected_cannot_be_probed() {
    let status = ProviderStatus::new(MetricsMode::Auto, ProviderState::WaitingForCluster);
    let m = model(&status, false);
    assert_eq!(m.headline.text, "Waiting for the cluster to connect");
    assert!(!m.can_redetect);
    assert!(m.unavailable.unwrap().contains("not connected"));
}

#[test]
fn a_manual_endpoint_shows_its_settings_and_only_the_name_of_the_keychain_entry() {
    let m = model(&manual(), true);
    let value = |label: &str| {
        m.settings
            .iter()
            .find(|line| line.label == label)
            .map(|line| line.value.clone())
    };
    assert_eq!(value("Provider").as_deref(), Some("manual"));
    assert_eq!(value("Stack").as_deref(), Some("VictoriaMetrics"));
    assert_eq!(value("URL").as_deref(), Some("https://vm.example.com"));
    assert_eq!(
        value("Path prefix").as_deref(),
        Some("/select/0/prometheus")
    );
    assert_eq!(
        value("Authentication").as_deref(),
        Some("bearer token in keychain entry prometheus/prod-vm")
    );
    assert_eq!(
        m.probes_note,
        "A configured endpoint is checked as it is; nothing is searched for."
    );
    assert_eq!(m.headline.tone, Tone::Good);
}

#[test]
fn auto_mode_lists_only_its_two_settings_and_says_where_to_edit() {
    let m = model(&found(), true);
    let labels: Vec<_> = m.settings.iter().map(|l| l.label).collect();
    assert_eq!(labels, ["Enabled", "Provider"]);
    assert_eq!(
        m.edit_hint,
        format!(
            "Change these in settings.json under clusters.{}.prometheus.",
            cluster()
        )
    );
}

#[test]
fn a_finished_run_shows_when_it_finished() {
    let mut status = found();
    status.checked_at = Some(jiff::Timestamp::from_second(1_791_707_595).unwrap());
    let m = model(&status, true);
    let time = m.checked.unwrap();
    assert_eq!(time.len(), 8, "HH:MM:SS: {time}");
    assert_eq!(model(&found(), true).checked, None);
}

#[test]
fn a_detection_with_no_probes_yet_says_what_it_is_doing() {
    let m = model(&detecting(), true);
    assert!(m.probes_note.starts_with("Listing the services"));
    let waiting = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::NotFound),
    );
    assert_eq!(
        model(&waiting, true).probes_note,
        "Nothing has been looked at yet."
    );
}

#[test]
fn a_configured_endpoint_that_fails_is_named_as_such() {
    use oxikube_domain::MetricsProviderKind;
    use oxikube_ports::{DetectionFailure, ProviderTarget};

    let failed = |failure| {
        ProviderStatus::new(
            MetricsMode::Manual,
            ProviderState::Detection(DetectionState::Failed(failure)),
        )
    };
    let probe = model(
        &failed(DetectionFailure::ProbeFailed {
            kind: MetricsProviderKind::VictoriaMetrics,
            target: ProviderTarget::Url("https://vm.example.com".into()),
            reason: "prometheus rejected the credentials: 401 Unauthorized".into(),
        }),
        true,
    );
    assert_eq!(
        probe.headline.text,
        "The configured endpoint did not answer"
    );
    assert!(
        probe
            .unavailable
            .unwrap()
            .contains("https://vm.example.com")
    );
    let config = model(
        &failed(DetectionFailure::Other(
            "manual mode needs a `prometheus.url` or a `prometheus.path`".into(),
        )),
        true,
    );
    assert_eq!(
        config.headline.text,
        "The configured endpoint cannot be used"
    );
    assert_eq!(config.headline.tone, Tone::Error);
}
