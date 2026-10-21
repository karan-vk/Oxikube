use oxikube_domain::OxiError;
use oxikube_domain::helm::HelmError;

use crate::helm_repos::describe_error;

fn failed(stderr: &str) -> OxiError {
    HelmError::CliFailed {
        exit_code: Some(1),
        stderr_summary: stderr.into(),
    }
    .into()
}

#[test]
fn helms_failures_are_classified_for_the_person_reading_them() {
    let cases = [
        (
            "looks like \"https://x\" is not a valid chart repository or cannot be reached: Get \"https://x/index.yaml\": dial tcp: lookup x: no such host",
            "could not be reached",
        ),
        (
            "failed to fetch https://x/index.yaml : 401 Unauthorized",
            "credentials",
        ),
        (
            "failed to fetch https://x/index.yaml : 404 Not Found",
            "404",
        ),
        ("x509: certificate signed by unknown authority", "CA file"),
        (
            "repository name (bitnami) already exists, please specify a different name",
            "already exists",
        ),
        ("Get \"https://x\": context deadline exceeded", "in time"),
        (
            "no repo named \"nope\" found",
            "no repository with that name",
        ),
        (
            "unable to read index.yaml: yaml: line 3: mapping values",
            "valid chart repository index",
        ),
    ];
    for (stderr, expect) in cases {
        let text = describe_error(&failed(stderr));
        assert!(text.contains(expect), "{stderr} -> {text}");
        assert!(text.contains(stderr), "the original text is kept: {text}");
    }
}

#[test]
fn a_missing_binary_is_shown_as_is() {
    let text = describe_error(&HelmError::BinaryMissing.into());
    assert!(text.contains("the helm binary was not found"), "{text}");
}

#[test]
fn unknown_failures_pass_through() {
    let text = describe_error(&failed("something odd"));
    assert!(
        text.contains("something odd") && !text.contains(". helm"),
        "{text}"
    );
}

#[test]
fn a_port_or_a_name_is_not_a_status_or_a_lock() {
    let port = describe_error(&failed(
        "Get \"https://host:8401/charts/index.yaml\": connection refused",
    ));
    assert!(
        port.contains("could not be reached") && !port.contains("refused the credentials"),
        "{port}"
    );
    let name = describe_error(&failed(
        "repository block-charts cannot be reached: no such host",
    ));
    assert!(
        name.contains("could not be reached") && !name.contains("locked"),
        "{name}"
    );
}
