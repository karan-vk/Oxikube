//! The pure parts: the form and its spec, the masked diffs.

use oxikube_domain::helm::{ChartRef, ValuesYaml};

use super::web;
use crate::upgrade::{ChartSource, UpgradeForm, defaults_diff, values_diff};

fn sources() -> Vec<ChartSource> {
    vec![
        ChartSource {
            chart: ChartRef::new("nginx", "").in_repo("bitnami"),
            versions: vec!["15.4.0".into(), "15.0.0".into()],
        },
        ChartSource {
            chart: ChartRef::new("nginx", "").in_repo("other"),
            versions: vec!["1.0.0".into()],
        },
    ]
}

#[test]
fn the_form_names_the_chart_of_a_repository_or_a_reference_as_typed() {
    let mut form = UpgradeForm {
        chart: " other/nginx ".into(),
        version: "1.0.0".into(),
        ..UpgradeForm::default()
    };
    let chart = form.chart_ref(&sources());
    assert_eq!(chart, ChartRef::new("nginx", "1.0.0").in_repo("other"));
    form.chart = "oci://registry.example/charts/nginx".into();
    form.version = String::new();
    let chart = form.chart_ref(&sources());
    assert_eq!(chart.repo, None);
    assert_eq!(chart.reference(), "oci://registry.example/charts/nginx");
}

#[test]
fn the_spec_carries_the_flags_and_the_form_blocks_what_helm_would_refuse() {
    let form = UpgradeForm {
        chart: "bitnami/nginx".into(),
        version: "15.4.0".into(),
        reuse_values: true,
        wait: true,
        timeout: "120s".into(),
    };
    let spec = form
        .spec(&web(), &sources(), ValuesYaml::new("a: 1\n"))
        .expect("valid");
    assert!(spec.reuse_values && spec.wait);
    assert_eq!(spec.timeout, Some(std::time::Duration::from_secs(120)));
    assert_eq!(spec.chart.reference(), "bitnami/nginx");
    assert_eq!(spec.chart.version, "15.4.0");

    let bad = UpgradeForm {
        chart: " ".into(),
        timeout: "soon".into(),
        ..form
    };
    let errors = bad
        .spec(&web(), &sources(), ValuesYaml::default())
        .unwrap_err();
    assert!(errors.chart.is_some() && errors.timeout.is_some());
}

#[test]
fn the_values_diff_shows_changes_and_fingerprints_secrets() {
    let current = "replicaCount: 2\nauth:\n  password: hunter2\n";
    let edited = "replicaCount: 3\nauth:\n  password: hunter3\n";
    let diff = values_diff(current, edited);
    assert!(!diff.is_empty());
    let (removed, added) = diff.counts();
    assert_eq!(
        (removed, added),
        (2, 2),
        "the replicas and the password changed"
    );
    let shown: String = (0..diff.inline_rows().len() as u32 * 2)
        .map(|i| format!("{}{}", diff.old_line(i), diff.new_line(i)))
        .collect();
    assert!(!shown.contains("hunter"), "{shown}");
    assert!(shown.contains("(hidden #"));
    assert!(values_diff(current, current).is_empty());
}

#[test]
fn the_defaults_diff_lists_added_and_renamed_keys() {
    let diff = defaults_diff(super::DEFAULTS_OLD, super::DEFAULTS_NEW);
    let (removed, added) = diff.counts();
    assert_eq!((removed, added), (1, 3), "port renamed, ingress added");
}
