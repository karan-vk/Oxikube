//! The install tab in a window: the form, the values editor, preview, confirm, stream, cancel.

use gpui::TestAppContext;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::helm::OutputLine;
use oxikube_testkit::HelmCall;
use oxikube_ui::editor::EditorAssist as _;

use super::{Harness, Options};
use crate::install::Stage;

#[gpui::test]
fn the_form_opens_with_the_charts_defaults_versions_and_namespaces(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Editing);
    let values = h.read(|v, cx| v.values(cx));
    assert!(values.expose().contains("replicaCount: 1"));
    let form = h.read(|v, cx| v.form(cx));
    assert_eq!(form.release, "nginx");
    assert_eq!(form.namespace, "default");
    assert_eq!(form.version, "15.4.0");
    assert_eq!(h.read(|v, _| v.versions.clone()), ["15.4.0", "15.3.1"]);
    assert_eq!(
        h.read(|v, _| v.namespaces.clone()),
        Some(vec![
            "default".to_owned(),
            "demo".to_owned(),
            "kube-system".to_owned()
        ])
    );
    for selector in [
        "helm-install-release",
        "helm-install-namespace",
        "helm-install-create-ns",
        "helm-install-versions",
        "helm-install-values",
        "helm-install-preview",
        "helm-install-install",
        "helm-install-preview-pane",
    ] {
        assert!(h.is_laid_out(selector), "{selector}");
    }
}

#[gpui::test]
fn the_values_editor_documents_keys_from_the_charts_comments(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    let (assist, snapshot) = h.read(|v, cx| (v.assist().clone(), v.editor.read(cx).snapshot(cx)));
    let text = snapshot.to_text();
    let at = text.find("replicaCount").unwrap() + 3;
    let hover = assist.hover(&snapshot, at).expect("documented");
    assert!(
        hover.markdown.contains("**replicaCount** `int`"),
        "{}",
        hover.markdown
    );
    assert!(hover.markdown.contains("Number of replicas"));
    let repo = text.find("repository:").unwrap() + 2;
    let hover = assist.hover(&snapshot, repo).unwrap();
    assert!(
        hover.markdown.contains("Image repository"),
        "{}",
        hover.markdown
    );
}

#[gpui::test]
fn late_defaults_never_replace_what_the_user_typed(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    // The version when a (slow) defaults read starts; the user then types before it lands.
    let baseline = h.read(|v, cx| v.values_version(cx));
    h.set_values("replicaCount: 3\n");
    h.update(|view, window, cx| view.apply_defaults(super::DEFAULTS, baseline, window, cx));
    assert_eq!(h.read(|v, cx| v.values(cx)).expose(), "replicaCount: 3\n");

    // Untouched since the read started: the defaults go in.
    let baseline = h.read(|v, cx| v.values_version(cx));
    h.update(|view, window, cx| view.apply_defaults(super::DEFAULTS, baseline, window, cx));
    assert_eq!(h.read(|v, cx| v.values(cx)).expose(), super::DEFAULTS);
}

#[gpui::test]
fn an_invalid_release_name_blocks_the_preview(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.set_release("Not_Valid");
    assert!(h.is_laid_out("helm-install-release-problem"));
    h.update(|view, window, cx| view.preview(window, cx));
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Editing);
    assert!(
        !h.fake
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, HelmCall::Template(_)))
    );
}

#[gpui::test]
fn preview_shows_the_masked_manifest_then_install_asks_and_streams(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.set_release("web");
    h.set_values("replicaCount: 2\nauth:\n  password: hunter2-value\n");
    h.script_preview();
    h.click("helm-install-preview");

    let manifest = h.read(|v, cx| v.manifest_text(cx));
    assert!(manifest.contains("kind: Service"), "{manifest}");
    assert!(
        !manifest.contains("c3VwZXJzZWNyZXQ="),
        "the Secret's data is masked"
    );
    assert!(h.is_laid_out("helm-install-manifest"));
    assert!(h.read(|v, cx| v.fresh_preview(cx).is_some()));

    h.click("helm-install-install");
    let Stage::Confirming(request) = h.read(|v, _| v.stage().clone()) else {
        panic!("the guard asks first");
    };
    assert!(
        request.summary.contains("bitnami/nginx 15.4.0"),
        "{}",
        request.summary
    );
    assert!(h.is_laid_out("helm-install-confirm"));
    // Only the dry run so far.
    let installs = || {
        move |h: &Harness| {
            h.fake
                .recorded_calls()
                .into_iter()
                .filter_map(|c| match c {
                    HelmCall::Install(r) => Some(r),
                    _ => None,
                })
                .collect::<Vec<_>>()
        }
    };
    assert!(installs()(&h).iter().all(|r| r.dry_run));

    h.fake.script().install.push_ok(vec![
        OutputLine::stdout("NAME: web"),
        OutputLine::stderr("coalesce.go: warning"),
        OutputLine::stdout("STATUS: deployed"),
    ]);
    h.click("helm-install-confirm-yes");
    assert_eq!(
        h.read(|v, _| v.stage().clone()),
        Stage::Installed("Installed release web in namespace default".into())
    );
    assert_eq!(
        h.read(|v, _| v.log().text()),
        "NAME: web\n! coalesce.go: warning\nSTATUS: deployed\n"
    );
    assert!(h.is_laid_out("helm-install-log"));
    let last = installs()(&h).pop().unwrap();
    assert!(!last.dry_run);
    assert_eq!(last.release, "web");
    // Audited: the dry run and the install, never the values.
    let audit = h.audit();
    assert_eq!(audit.len(), 2);
    assert!(audit[0].dry_run && !audit[1].dry_run);
    assert_eq!(audit[1].outcome, AuditOutcome::Succeeded);
    assert!(!format!("{audit:?}").contains("hunter2"));
}

#[gpui::test]
fn an_edit_after_the_preview_makes_it_stale(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.script_preview();
    h.click("helm-install-preview");
    assert!(h.read(|v, cx| v.fresh_preview(cx).is_some()));
    h.set_values("replicaCount: 3\n");
    assert!(h.read(|v, cx| v.fresh_preview(cx).is_none()));
    h.update(|view, _, cx| view.install(cx));
    assert_eq!(
        h.read(|v, _| v.stage().clone()),
        Stage::Editing,
        "install needs a fresh preview"
    );
}

#[gpui::test]
fn a_server_refusal_is_shown_inline_and_blocks_install(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.fake.script().template.push_ok(super::RENDERED.to_owned());
    h.fake
        .script()
        .install
        .push_err(oxikube_domain::OxiError::internal(
            "cannot re-use a name that is still in use",
        ));
    h.click("helm-install-preview");
    assert!(h.is_laid_out("helm-install-server-error"));
    h.update(|view, _, cx| view.install(cx));
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Editing);
}

#[gpui::test]
fn a_template_error_is_shown_inline(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.fake
        .script()
        .template
        .push_err(oxikube_domain::OxiError::validation(
            "values don't meet the schema",
        ));
    h.fake.script().install.push_ok(Vec::new());
    h.click("helm-install-preview");
    assert!(h.is_laid_out("helm-install-preview-error"));
    assert!(
        h.read(|v, _| v.preview_error.clone())
            .unwrap()
            .contains("schema")
    );
}

#[gpui::test]
fn cancel_stops_helm(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.script_preview();
    h.click("helm-install-preview");
    h.click("helm-install-install");
    h.fake.hold_streams(true);
    h.fake
        .script()
        .install
        .push_ok(vec![OutputLine::stdout("waiting for resources")]);
    h.click("helm-install-confirm-yes");
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Installing);
    assert_eq!(h.fake.open_streams(), 1);
    assert_eq!(h.read(|v, _| v.log().text()), "waiting for resources\n");
    h.click("helm-install-cancel");
    assert_eq!(h.fake.open_streams(), 0, "the helm process is killed");
    assert!(
        matches!(h.read(|v, _| v.stage().clone()), Stage::Failed(why) if why.starts_with("Cancelled"))
    );
}

#[gpui::test]
fn declining_the_confirmation_returns_to_the_form(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.script_preview();
    h.click("helm-install-preview");
    h.click("helm-install-install");
    h.click("helm-install-confirm-no");
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Editing);
    let outcomes: Vec<_> = h.audit().iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [AuditOutcome::Succeeded, AuditOutcome::Cancelled]);
}

#[gpui::test]
fn a_read_only_cluster_refuses_the_preview(cx: &mut TestAppContext) {
    let mut h = Harness::open_with(
        cx,
        Options {
            read_only: true,
            ..Options::default()
        },
    );
    h.click("helm-install-preview");
    let error = h.read(|v, _| v.preview_error.clone()).expect("refused");
    assert!(error.contains("read-only"), "{error}");
    assert!(
        !h.fake
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, HelmCall::Template(_) | HelmCall::Install(_)))
    );
}

#[gpui::test]
fn without_helm_the_tab_shows_guidance_only(cx: &mut TestAppContext) {
    let mut h = Harness::open_with(
        cx,
        Options {
            with_helm: false,
            ..Options::default()
        },
    );
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::NoHelm);
    assert!(h.is_laid_out("helm-install-guidance"));
    assert!(!h.is_laid_out("helm-install-preview"));
    assert!(!h.is_laid_out("helm-install-values"));
}
