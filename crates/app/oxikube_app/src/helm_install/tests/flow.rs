//! The install sequence: preview, ask, start (streamed), decline, cancel, read-only.

use futures::FutureExt as _;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::helm::ValuesYaml;
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_testkit::HelmCall;

use super::{Setup, line, now, spec};
use crate::helm_install::InstallError;

#[test]
fn preview_renders_masks_secrets_and_asks_the_server_without_installing() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).expect("previewed");

    assert!(preview.can_install());
    assert_eq!(preview.summary.describe(), "2 objects: 1 Secret, 1 Service");
    assert!(preview.manifest.contains("kind: Service"));
    assert!(
        !preview.manifest.contains("c3VwZXJzZWNyZXQ="),
        "{}",
        preview.manifest
    );
    // The template renders, the server only rehearses: no real install was asked for.
    let calls = s.helm.recorded_calls();
    assert!(matches!(&calls[0], HelmCall::Template(r) if !r.dry_run && r.create_namespace));
    assert!(matches!(&calls[1], HelmCall::Install(r) if r.dry_run));
    assert!(s.installs().iter().all(|r| r.dry_run));
    // Audited once, as a dry run, naming the release and never the values.
    let audit = s.audit_of_installs();
    assert_eq!(audit.len(), 1);
    assert!(audit[0].dry_run);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(&*audit[0].target.name, "web");
    assert_eq!(audit[0].target.namespace(), Some("demo"));
    assert_eq!(&*audit[0].target.gvk.kind, "Release");
}

#[test]
fn a_server_refusal_shows_in_the_preview_and_blocks_the_install() {
    let s = Setup::new(false);
    s.helm.script().template.push_ok(super::RENDERED.to_owned());
    s.helm
        .script()
        .install
        .push_err(oxikube_domain::OxiError::internal(
            "cannot re-use a name that is still in use",
        ));
    let preview = now(s.flow.preview(spec())).expect("previewed");
    assert!(!preview.can_install());
    assert!(
        preview
            .server_error
            .as_deref()
            .is_some_and(|e| e.contains("still in use"))
    );
    assert!(matches!(
        now(s.flow.ask(&preview)),
        Err(InstallError::Invalid(_))
    ));
    assert!(s.installs().iter().all(|r| r.dry_run));
}

#[test]
fn a_template_failure_is_the_previews_error() {
    let s = Setup::new(false);
    s.helm
        .script()
        .template
        .push_err(oxikube_domain::OxiError::validation(
            "values don't meet the specifications of the schema",
        ));
    s.helm.script().install.push_ok(Vec::new());
    let error = now(s.flow.preview(spec())).unwrap_err();
    assert!(
        matches!(&error, InstallError::Failed(m) if m.contains("schema")),
        "{error:?}"
    );
    assert_eq!(s.audit_of_installs()[0].outcome, AuditOutcome::Failed);
}

#[test]
fn values_that_are_not_a_map_are_refused_before_anything_runs() {
    let s = Setup::new(false);
    let mut bad = spec();
    bad.values = ValuesYaml::new("image: [unclosed\n");
    assert!(matches!(
        now(s.flow.preview(bad)),
        Err(InstallError::Invalid(_))
    ));
    assert!(s.helm.recorded_calls().is_empty());
    assert!(s.audit_of_installs().is_empty());
}

#[test]
fn confirm_low_after_preview_then_the_output_streams_in_order() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).unwrap();
    let asked = now(s.flow.ask(&preview)).expect("a confirmation");
    assert_eq!(asked.tier, ConfirmTier::Simple);
    assert_eq!(asked.risk, Some(Risk::Low));
    assert!(
        asked.summary.contains("bitnami/nginx 15.4.0"),
        "{}",
        asked.summary
    );
    assert!(asked.summary.contains("release web in namespace demo"));
    assert!(!asked.summary.contains("hunter2"));
    // Asking installs nothing.
    assert_eq!(s.installs().len(), 1);

    s.helm.script().install.push_ok(vec![
        line("NAME: web"),
        line("STATUS: deployed"),
        line("REVISION: 1"),
    ]);
    let mut start = s.flow.start(&preview, &asked);
    let message = now(start.run).expect("installed");
    assert_eq!(message, "Installed release web in namespace demo");
    let mut lines = Vec::new();
    while let Ok(next) = start.lines.try_recv() {
        lines.push(next.text);
    }
    assert_eq!(lines, ["NAME: web", "STATUS: deployed", "REVISION: 1"]);

    let installs = s.installs();
    let real = installs.last().unwrap();
    assert!(!real.dry_run && real.create_namespace);
    assert_eq!(real.values.get("replicaCount"), Some(&serde_json::json!(2)));
    let audit = s.audit_of_installs();
    let last = audit.last().unwrap();
    assert!(!last.dry_run);
    assert_eq!(last.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        last.detail.as_deref(),
        Some("chart=bitnami/nginx version=15.4.0 create_namespace=true wait=false values=given")
    );
}

#[test]
fn the_audit_never_holds_the_values() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).unwrap();
    let asked = now(s.flow.ask(&preview)).unwrap();
    s.helm.script().install.push_ok(vec![line("done")]);
    now(s.flow.start(&preview, &asked).run).unwrap();
    let all = format!("{:?}", s.h.audit());
    assert!(!all.contains("hunter2"), "{all}");
    assert!(!all.contains("replicaCount"), "{all}");
}

#[test]
fn declining_installs_nothing_and_is_audited_as_cancelled() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).unwrap();
    let asked = now(s.flow.ask(&preview)).unwrap();
    now(s.flow.decline(asked));
    assert!(s.installs().iter().all(|r| r.dry_run));
    let audit = s.audit_of_installs();
    assert_eq!(audit.last().unwrap().outcome, AuditOutcome::Cancelled);
}

#[test]
fn cancel_kills_the_stream_and_is_audited() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).unwrap();
    let asked = now(s.flow.ask(&preview)).unwrap();
    s.helm.hold_streams(true);
    s.helm
        .script()
        .install
        .push_ok(vec![line("waiting for resources")]);
    let mut start = s.flow.start(&preview, &asked);
    assert!(
        (&mut start.run).now_or_never().is_none(),
        "helm is still running"
    );
    assert_eq!(s.helm.open_streams(), 1);
    assert_eq!(
        start.lines.try_recv().map(|l| l.text).ok().as_deref(),
        Some("waiting for resources")
    );
    // Cancel: the dispatch is dropped, so is the stream (the process is killed).
    drop(start.run);
    assert_eq!(s.helm.open_streams(), 0);
    // The followers' stream ends with the install.
    assert!(matches!(
        start.lines.try_recv(),
        Err(futures::channel::mpsc::TryRecvError::Closed)
    ));
    // The audit lands as the next mutation flushes the backlog: preview again.
    s.helm.hold_streams(false);
    s.script_preview();
    now(s.flow.preview(spec())).unwrap();
    let outcomes: Vec<_> = s.audit_of_installs().iter().map(|r| r.outcome).collect();
    assert!(outcomes.contains(&AuditOutcome::Cancelled), "{outcomes:?}");
}

#[test]
fn read_only_blocks_the_preview_and_the_install() {
    let s = Setup::new(true);
    let error = now(s.flow.preview(spec())).unwrap_err();
    assert!(matches!(error, InstallError::ReadOnly { .. }), "{error:?}");
    assert!(s.helm.recorded_calls().is_empty());
    let outcomes: Vec<_> = s.audit_of_installs().iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [AuditOutcome::Denied]);
}
