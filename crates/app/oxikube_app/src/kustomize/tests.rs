use std::path::Path;
use std::sync::Arc;

use futures::executor::block_on;
use oxikube_domain::ErrorKind;
use oxikube_domain::command::Capabilities;
use oxikube_domain::kustomize::{BuildOutput, KustomizeTool, KustomizeToolKind};
use oxikube_testkit::fakes::{FakeKustomizePort, KustomizeCall};

use super::*;

fn tool() -> KustomizeTool {
    KustomizeTool {
        kind: KustomizeToolKind::Kubectl,
        version: "v5.0.4".into(),
        path: "/usr/local/bin/kubectl".into(),
    }
}

const BUILT: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: demo-app\n---\n\
apiVersion: v1\nkind: Secret\nmetadata:\n  name: demo-creds\nstringData:\n  password: hunter2\n";

#[test]
fn a_build_parses_under_the_kustomize_label_and_keeps_stderr() {
    let fake = Arc::new(FakeKustomizePort::with_tool(tool()));
    fake.script().build.push_ok(BuildOutput {
        yaml: BUILT.into(),
        stderr: "warning: deprecated field".into(),
    });
    let service = KustomizeService::new(fake.clone());
    let built = block_on(service.build(Path::new("/o/prod"))).expect("build");
    assert_eq!(built.docs.len(), 2);
    assert_eq!(built.docs[0].source.as_str(), "kustomize:/o/prod");
    assert_eq!(built.docs[1].line, 6, "lines point into the built output");
    assert!(built.errors.is_empty());
    assert_eq!(built.stderr, "warning: deprecated field");
    assert_eq!(built.tool, tool());
    assert_eq!(built.dir, Path::new("/o/prod"));
    assert!(
        fake.recorded_calls()
            .contains(&KustomizeCall::Build("/o/prod".into()))
    );
}

#[test]
fn the_debug_of_a_build_never_shows_secret_values() {
    let fake = Arc::new(FakeKustomizePort::with_tool(tool()));
    fake.script().build.push_ok(BuildOutput {
        yaml: BUILT.into(),
        stderr: String::new(),
    });
    let built = block_on(KustomizeService::new(fake).build(Path::new("/o"))).expect("build");
    let shown = format!("{built:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(shown.contains("demo-creds"));
}

#[test]
fn a_bad_document_is_an_error_beside_the_good_ones() {
    let fake = Arc::new(FakeKustomizePort::with_tool(tool()));
    fake.script().build.push_ok(BuildOutput {
        yaml: format!("{BUILT}---\n: : bad\n- x\n"),
        stderr: String::new(),
    });
    let built = block_on(KustomizeService::new(fake).build(Path::new("/o"))).expect("build");
    assert_eq!(built.docs.len(), 2);
    assert_eq!(built.errors.len(), 1);
    assert_eq!(built.errors[0].source.as_str(), "kustomize:/o");
}

#[test]
fn no_tool_hides_the_capability_and_refuses_builds() {
    let fake = Arc::new(FakeKustomizePort::new());
    let service = KustomizeService::new(fake.clone());
    assert_eq!(block_on(service.tool()), None);
    let base = Capabilities::MUTATE | Capabilities::KUSTOMIZE;
    assert_eq!(block_on(service.capabilities(base)), Capabilities::MUTATE);
    let error = block_on(service.build(Path::new("/o"))).expect_err("no tool");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        !fake
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, KustomizeCall::Build(_)))
    );

    fake.set_tool(Some(tool()));
    assert!(block_on(service.refresh()).is_some());
    assert!(block_on(service.capabilities(Capabilities::MUTATE)).contains(Capabilities::KUSTOMIZE));
}

#[test]
fn a_failed_build_surfaces_stderr_in_the_error() {
    let fake = Arc::new(FakeKustomizePort::with_tool(tool()));
    fake.script()
        .build
        .push(Err(oxikube_domain::kustomize::KustomizeError::Failed {
            exit_code: Some(1),
            stderr: "Error: accumulating resources".into(),
        }
        .into()));
    let error = block_on(KustomizeService::new(fake).build(Path::new("/o"))).expect_err("fails");
    assert!(error.message().contains("accumulating resources"));
}
