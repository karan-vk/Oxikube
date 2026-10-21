//! The form's rules, the completion site, the output log.

use oxikube_domain::helm::{ChartRef, OutputLine, ValuesYaml};

use super::cluster;
use crate::install::{InstallForm, MAX_LOG_LINES, OutputLog, Site, site_at};

fn form() -> InstallForm {
    InstallForm {
        release: "web".into(),
        namespace: "demo".into(),
        ..InstallForm::default()
    }
}

#[test]
fn release_and_namespace_follow_helms_rules() {
    let known = vec!["default".to_owned(), "demo".to_owned()];
    assert!(form().errors(Some(&known)).is_empty());
    let mut bad = form();
    bad.release = "Web_App".into();
    assert!(bad.errors(Some(&known)).release.is_some());
    bad.release = "a".repeat(54);
    assert!(bad.errors(Some(&known)).release.unwrap().contains("53"));
    bad.release = "web".into();
    bad.namespace = "Demo".into();
    assert!(bad.errors(Some(&known)).namespace.is_some());
}

#[test]
fn a_new_namespace_needs_create() {
    let known = vec!["default".to_owned()];
    let mut f = form();
    assert!(f.namespace_is_new(Some(&known)));
    let error = f.errors(Some(&known)).namespace.expect("asks to create it");
    assert!(error.contains("Create namespace"), "{error}");
    f.create_namespace = true;
    assert!(f.errors(Some(&known)).is_empty());
    // While the namespaces are unknown, nothing is claimed about them.
    f.create_namespace = false;
    assert!(f.errors(None).is_empty());
    assert!(!f.namespace_is_new(None));
}

#[test]
fn timeout_is_seconds_and_the_spec_carries_the_form() {
    let mut f = form();
    f.timeout = "abc".into();
    assert!(f.errors(None).timeout.is_some());
    f.timeout = "120s".into();
    f.wait = true;
    f.version = "15.3.1".into();
    let chart = ChartRef::new("nginx", "15.4.0").in_repo("bitnami");
    let spec = f
        .spec(&cluster(), &chart, ValuesYaml::new("a: 1\n"), None)
        .expect("valid");
    assert_eq!(spec.chart.version, "15.3.1");
    assert_eq!(spec.timeout, Some(std::time::Duration::from_secs(120)));
    assert!(spec.wait);
    assert_eq!(spec.release, "web");
    f.release.clear();
    assert!(
        f.spec(&cluster(), &chart, ValuesYaml::default(), None)
            .is_err()
    );
}

#[test]
fn the_completion_site_reads_keys_and_values_off_the_lines() {
    let text = "image:\n  repository: x\n  ta\nservice:\n  type: Cl\nrep";
    let key = text.find("ta\n").unwrap() + 2;
    assert_eq!(
        site_at(text, key),
        Some(Site::Key {
            parent: "image".into(),
            word: key - 2..key
        })
    );
    let value = text.find("Cl").unwrap() + 2;
    assert_eq!(
        site_at(text, value),
        Some(Site::Value {
            path: "service.type".into(),
            word: value - 2..value
        })
    );
    let top = text.len();
    assert_eq!(
        site_at(text, top),
        Some(Site::Key {
            parent: String::new(),
            word: top - 3..top
        })
    );
    assert_eq!(site_at("# comm", 6), None);
    assert_eq!(site_at("list:\n  - na", 12), None);
}

#[test]
fn the_log_keeps_the_newest_lines() {
    let mut log = OutputLog::default();
    for i in 0..MAX_LOG_LINES + 5 {
        log.push(OutputLine::stdout(format!("line {i}")));
    }
    assert_eq!(log.len(), MAX_LOG_LINES);
    assert_eq!(log.dropped(), 5);
    assert_eq!(log.get(0).unwrap().text, "line 5");
    log.push(OutputLine::stderr("warn"));
    assert!(log.text().ends_with("! warn\n"));
}
