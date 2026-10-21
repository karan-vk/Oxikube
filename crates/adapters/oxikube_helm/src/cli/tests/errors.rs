//! Error mapping over recorded Helm 3.10 / 4.3 stderr, and redaction.

use oxikube_domain::ErrorKind;
use oxikube_domain::helm::Values;
use oxikube_domain::redact::MARKER;

use super::{VERSIONS, fixture};
use crate::cli::errors::{CliFailure, Redactor, classify, summary};

fn kind_of(version: &str, name: &str) -> (ErrorKind, String) {
    let text = summary(&fixture(version, name), &Redactor::none());
    let error = classify(Some(1), &text, Some("rec-a"));
    (error.kind(), error.message().to_owned())
}

#[test]
fn recorded_failures_map_onto_the_taxonomy() {
    for version in VERSIONS {
        for (name, kind) in [
            ("history_missing.err", ErrorKind::NotFound),
            ("values_missing.err", ErrorKind::NotFound),
            ("uninstall_missing.err", ErrorKind::NotFound),
            ("show_missing.err", ErrorKind::NotFound),
            ("show_version_missing.err", ErrorKind::NotFound),
            ("install_inuse.err", ErrorKind::Conflict),
            ("badctx.err", ErrorKind::NotFound),
            ("unreachable.err", ErrorKind::Network),
        ] {
            let (got, message) = kind_of(version, name);
            assert_eq!(got, kind, "{version}/{name}: {message}");
        }
        let (_, message) = kind_of(version, "history_missing.err");
        assert!(message.contains("rec-a"), "names the release: {message}");
        let (_, message) = kind_of(version, "show_missing.err");
        assert!(message.starts_with("chart not found"), "{message}");
    }
}

#[test]
fn summary_keeps_helms_error_lines_only() {
    // Helm 4 logs a `level=ERROR` line before its `Error:` line.
    let text = summary(&fixture("helm-4.3", "install_inuse.err"), &Redactor::none());
    assert_eq!(
        text,
        "INSTALLATION FAILED: release name check failed: cannot reuse a name that is still in use"
    );
    // Helm 3 prints kubeconfig permission warnings first.
    let text = summary(&fixture("helm-3.10", "unreachable.err"), &Redactor::none());
    assert!(text.starts_with("Kubernetes cluster unreachable"), "{text}");
    assert!(!text.contains("WARNING"));
    // Without an `Error:` line, the last lines (warnings dropped).
    let text = summary("WARNING: x\nfirst\nsecond\n", &Redactor::none());
    assert_eq!(text, "first; second");
    let long = format!("Error: {}", "x".repeat(2000));
    assert!(summary(&long, &Redactor::none()).chars().count() <= 601);
}

#[test]
fn other_failures() {
    let kind = |s: &str| classify(Some(1), s, None).kind();
    assert_eq!(
        kind("pods is forbidden: User \"bob\" cannot list"),
        ErrorKind::Forbidden
    );
    assert_eq!(kind("Unauthorized"), ErrorKind::Auth);
    assert_eq!(kind("unknown flag: --kube-server"), ErrorKind::Unsupported);
    assert_eq!(kind("something odd"), ErrorKind::Internal);
    assert_eq!(
        CliFailure::Timeout(std::time::Duration::from_secs(30))
            .into_error(None)
            .kind(),
        ErrorKind::Timeout
    );
    let failure = CliFailure::Exit {
        code: Some(1),
        summary: "no repositories to show".into(),
    };
    assert!(failure.says("no repositories to show"));
}

#[test]
fn supplied_values_and_secret_patterns_are_blanked() {
    let values = Values::from_yaml(
        "db:\n  password: s3cr3t-hunter2\n  port: 54321\nreplicas: 2\nenabled: true\nshort: ab\n",
    )
    .unwrap();
    let redactor = Redactor::for_values(&values);
    let line = "Error: template: bad value s3cr3t-hunter2 on 54321 with 2 replicas, ab";
    let out = redactor.apply(line);
    assert!(!out.contains("s3cr3t-hunter2"), "{out}");
    assert!(!out.contains("54321"), "{out}");
    assert!(out.contains(MARKER));
    assert!(
        out.contains("with 2 replicas, ab"),
        "short values stay: {out}"
    );
    // The shared patterns apply even without values.
    let out = Redactor::none().apply("password: hunter2-not-supplied");
    assert!(!out.contains("hunter2-not-supplied"), "{out}");
}
