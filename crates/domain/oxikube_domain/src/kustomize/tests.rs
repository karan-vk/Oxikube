use super::*;
use crate::ErrorKind;
use crate::command::Capabilities;
use crate::error::OxiError;

fn tool() -> KustomizeTool {
    KustomizeTool {
        kind: KustomizeToolKind::Standalone,
        version: "v5.4.2".into(),
        path: "/usr/bin/kustomize".into(),
    }
}

#[test]
fn the_capability_follows_detection() {
    let session = Capabilities::MUTATE | Capabilities::KUSTOMIZE;
    assert_eq!(apply_to(session, None), Capabilities::MUTATE);
    assert_eq!(
        apply_to(Capabilities::MUTATE, Some(&tool())),
        session,
        "a detected tool grants it"
    );
}

#[test]
fn the_tool_shows_command_and_version() {
    assert_eq!(tool().to_string(), "kustomize build v5.4.2");
    let kubectl = KustomizeTool {
        kind: KustomizeToolKind::Kubectl,
        ..tool()
    };
    assert_eq!(kubectl.to_string(), "kubectl kustomize v5.4.2");
}

#[test]
fn build_output_debug_omits_the_manifest_text() {
    let output = BuildOutput {
        yaml: "kind: Secret\nstringData:\n  password: hunter2\n".into(),
        stderr: "warning".into(),
    };
    let shown = format!("{output:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(shown.contains("warning"));
}

#[test]
fn errors_map_onto_the_taxonomy_and_keep_stderr() {
    let failed = KustomizeError::Failed {
        exit_code: Some(1),
        stderr: "accumulating resources: missing".into(),
    };
    assert_eq!(failed.stderr(), Some("accumulating resources: missing"));
    let error = OxiError::from(failed);
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(error.message().contains("accumulating resources"));
    assert_eq!(
        OxiError::from(KustomizeError::NotInstalled).kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        OxiError::from(KustomizeError::Timeout("60s".into())).kind(),
        ErrorKind::Timeout
    );
    assert_eq!(KustomizeError::Cancelled.stderr(), None);
}
