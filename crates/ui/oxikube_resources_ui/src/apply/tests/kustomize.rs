//! The kustomize flow: the action is there only with a tool, the build's tool and version and
//! stderr are shown, a failed build shows its message, and built objects open as YAML.

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_domain::kustomize::BuildOutput;

use super::{CM_A, Rig, tool};
use crate::apply::{SourceKind, Stage};

#[gpui::test]
fn the_kustomize_action_is_hidden_without_a_tool_and_shown_with_one(cx: &mut TestAppContext) {
    let mut without = Rig::new(cx, None);
    without.open("/nowhere");
    assert!(without.is_laid_out("apply-pick-files"));
    assert!(!without.is_laid_out("apply-pick-kustomize"));
    assert_eq!(without.read(|v| v.kustomize_tool().cloned()), None);
}

#[gpui::test]
fn the_kustomize_action_shows_with_a_tool(cx: &mut TestAppContext) {
    let mut with = Rig::new(cx, Some(tool()));
    with.open("/nowhere");
    assert!(with.is_laid_out("apply-pick-kustomize"));
    assert_eq!(with.read(|v| v.kustomize_tool().cloned()), Some(tool()));
}

#[gpui::test]
fn a_build_continues_as_a_normal_plan_and_shows_the_tool_and_its_stderr(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, Some(tool()));
    rig.kustomize.script().build.push_ok(BuildOutput {
        yaml: CM_A.to_owned(),
        stderr: "warning: 'patchesStrategicMerge' is deprecated".to_owned(),
    });
    rig.request(SourceKind::Kustomize, Some("/overlay"));
    assert_eq!(rig.words(), ["created"]);
    assert!(rig.read(|v| v.is_kustomize()));
    rig.click("apply-stage-load");
    assert_eq!(rig.read(|v| v.model().stage()), Stage::Load);
    assert!(rig.is_laid_out("apply-kustomize-tool"));
    assert!(rig.is_laid_out("apply-kustomize-stderr"));
    let line = rig.read(|v| v.model().summary().unwrap().tool.clone());
    assert_eq!(line.as_deref(), Some("kustomize build v5.4.2"));
    // The rows say where they came from.
    let source = rig.read(|v| v.model().rows()[0].source.clone());
    assert_eq!(source, "kustomize:/overlay");
}

#[gpui::test]
fn a_failed_build_shows_kustomizes_stderr_and_nothing_is_planned(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, Some(tool()));
    rig.kustomize.script().build.push_err(OxiError::validation(
        "kustomize build failed: accumulating resources: '/overlay/missing' does not exist",
    ));
    rig.request(SourceKind::Kustomize, Some("/overlay"));
    assert!(rig.is_laid_out("apply-problem"));
    let text = rig.read(|v| v.problem.clone().unwrap_or_default());
    assert!(text.contains("accumulating resources"), "{text}");
    assert!(rig.read(|v| v.model().rows().is_empty()));
    assert!(rig.writes().is_empty());
}

#[gpui::test]
fn a_built_object_opens_in_the_editor_as_yaml_from_its_first_line(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, Some(tool()));
    rig.kustomize.script().build.push_ok(BuildOutput {
        yaml: CM_A.to_owned(),
        stderr: String::new(),
    });
    rig.request(SourceKind::Kustomize, Some("/overlay"));
    rig.click("apply-open-0");
    let opened = rig.opened.borrow().clone();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].line, 1);
    assert_eq!(opened[0].title, "configmap-a.yaml");
    assert!(opened[0].text.contains("name: a"), "{}", opened[0].text);
}
