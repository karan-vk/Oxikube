//! The upgrade sequence: preview, ask, start (streamed), decline, cancel, read-only.

use futures::FutureExt as _;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::helm::{ReleaseStatus, ValuesYaml};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_testkit::HelmCall;

use super::{Setup, line, now, revision, spec, values, web};
use crate::helm_upgrade::UpgradeError;

#[test]
fn preview_renders_masks_secrets_diffs_the_objects_and_asks_the_server_without_upgrading() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).expect("previewed");

    assert!(preview.can_upgrade());
    assert_eq!(preview.from_chart, "nginx-15.0.0");
    let named = |keys: &[oxikube_domain::helm::ObjectKey]| {
        keys.iter().map(ToString::to_string).collect::<Vec<_>>()
    };
    assert_eq!(
        named(&preview.changes.added),
        ["Ingress web", "Secret web-extra"]
    );
    assert_eq!(named(&preview.changes.changed), ["Deployment web"]);
    assert_eq!(
        named(&preview.changes.removed),
        ["Secret web-auth"],
        "{:?}",
        preview.changes
    );
    assert_eq!(preview.changes.unchanged, 1);
    assert!(preview.manifest.contains("kind: Ingress"));
    assert!(
        !preview.manifest.contains("c3VwZXJzZWNyZXQ="),
        "{}",
        preview.manifest
    );
    // The template renders the chart, the server only rehearses: nothing was upgraded.
    let calls = s.helm.recorded_calls();
    assert!(calls.iter().any(|c| matches!(c, HelmCall::Template(r)
        if !r.dry_run && r.values.get("replicaCount") == Some(&serde_json::json!(3)))));
    assert!(s.upgrades().iter().all(|r| r.dry_run));
    assert_eq!(s.upgrades().len(), 1);
    // Audited once, as a dry run, naming the release and the chart it ran; never the values.
    let audit = s.audit_of_upgrades();
    assert_eq!(audit.len(), 1);
    assert!(audit[0].dry_run);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(&*audit[0].target.name, "web");
    assert_eq!(audit[0].target.namespace(), Some("demo"));
    assert!(
        audit[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("from=nginx-15.0.0")
    );
}

#[test]
fn reuse_values_renders_the_previous_values_under_the_edits() {
    let s = Setup::new(false);
    s.script_preview();
    s.helm
        .script()
        .values
        .push_ok(values("replicaCount: 1\nimage:\n  tag: old\nextra: kept\n"));
    let mut reuse = spec();
    reuse.reuse_values = true;
    reuse.values = ValuesYaml::new("replicaCount: 5\n");
    now(s.flow.preview(reuse)).expect("previewed");
    let calls = s.helm.recorded_calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, HelmCall::Values { all: true, .. })),
        "the previous computed values are read"
    );
    let rendered = calls
        .iter()
        .find_map(|c| match c {
            HelmCall::Template(r) => Some(r.clone()),
            _ => None,
        })
        .expect("rendered");
    assert_eq!(
        rendered.values.get("replicaCount"),
        Some(&serde_json::json!(5))
    );
    assert_eq!(
        rendered.values.get("extra"),
        Some(&serde_json::json!("kept"))
    );
    assert_eq!(
        rendered.values.get("image.tag"),
        Some(&serde_json::json!("old"))
    );
    assert!(s.upgrades()[0].reuse_values);
}

#[test]
fn a_server_refusal_shows_in_the_preview_and_blocks_the_upgrade() {
    let s = Setup::new(false);
    let script = s.helm.script();
    script.manifest.push_ok(super::CURRENT.to_owned());
    script.history.push_ok(vec![revision(3, "15.0.0")]);
    script.template.push_ok(super::RENDERED.to_owned());
    script.upgrade.push_err(oxikube_domain::OxiError::internal(
        "another operation (install/upgrade/rollback) is in progress",
    ));
    let preview = now(s.flow.preview(spec())).expect("previewed");
    assert!(!preview.can_upgrade());
    assert!(
        preview
            .server_error
            .as_deref()
            .is_some_and(|e| e.contains("in progress"))
    );
    assert!(matches!(
        now(s.flow.ask(&preview)),
        Err(UpgradeError::Invalid(_))
    ));
    assert!(s.upgrades().iter().all(|r| r.dry_run));
}

#[test]
fn a_template_failure_is_the_previews_error() {
    let s = Setup::new(false);
    let script = s.helm.script();
    script.manifest.push_ok(super::CURRENT.to_owned());
    script.history.push_ok(vec![revision(3, "15.0.0")]);
    script
        .template
        .push_err(oxikube_domain::OxiError::validation(
            "values don't meet the specifications of the schema",
        ));
    script.upgrade.push_ok(Vec::new());
    let error = now(s.flow.preview(spec())).unwrap_err();
    assert!(
        matches!(&error, UpgradeError::Failed(m) if m.contains("schema")),
        "{error:?}"
    );
    assert_eq!(s.audit_of_upgrades()[0].outcome, AuditOutcome::Failed);
}

#[test]
fn values_that_are_not_a_map_are_refused_before_anything_runs() {
    let s = Setup::new(false);
    let mut bad = spec();
    bad.values = ValuesYaml::new("image: [unclosed\n");
    assert!(matches!(
        now(s.flow.preview(bad)),
        Err(UpgradeError::Invalid(_))
    ));
    assert!(s.helm.recorded_calls().is_empty());
    assert!(s.audit_of_upgrades().is_empty());
}

#[test]
fn confirm_medium_after_preview_then_the_output_streams_in_order() {
    let s = Setup::new(false);
    s.script_preview();
    s.helm.script().values.push_ok(values("replicaCount: 1\n"));
    let mut reuse = spec();
    reuse.reuse_values = true;
    let preview = now(s.flow.preview(reuse)).unwrap();
    let asked = now(s.flow.ask(&preview)).expect("a confirmation");
    assert_eq!(asked.tier, ConfirmTier::Simple);
    assert_eq!(asked.risk, Some(Risk::Medium));
    assert!(
        asked.summary.contains("bitnami/nginx 15.4.0"),
        "{}",
        asked.summary
    );
    assert!(asked.summary.contains("release web in namespace demo"));
    assert!(asked.summary.contains("reusing the previous values"));
    assert!(!asked.summary.contains("hunter2"));
    // Asking upgrades nothing.
    assert_eq!(s.upgrades().len(), 1);

    s.helm.script().history.push_ok(vec![revision(3, "15.0.0")]);
    s.helm.script().upgrade.push_ok(vec![
        line("Release \"web\" has been upgraded. Happy Helming!"),
        line("STATUS: deployed"),
        line("REVISION: 4"),
    ]);
    let mut start = s.flow.start(&preview, &asked);
    let message = now(start.run).expect("upgraded");
    assert_eq!(message, "Upgraded release web in namespace demo");
    let mut lines = Vec::new();
    while let Ok(next) = start.lines.try_recv() {
        lines.push(next.text);
    }
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2], "REVISION: 4");

    let upgrades = s.upgrades();
    let real = upgrades.last().unwrap();
    assert!(!real.dry_run && real.reuse_values);
    assert_eq!(real.values.get("replicaCount"), Some(&serde_json::json!(3)));
    let audit = s.audit_of_upgrades();
    let last = audit.last().unwrap();
    assert!(!last.dry_run);
    assert_eq!(last.outcome, AuditOutcome::Succeeded);
    let detail = last.detail.as_deref().unwrap();
    assert!(
        detail.contains("chart=bitnami/nginx version=15.4.0 reuse_values=true"),
        "{detail}"
    );
    assert!(detail.contains("from=nginx-15.0.0"), "{detail}");
    let all = format!("{:?}", s.h.audit());
    assert!(!all.contains("hunter2"), "{all}");
}

#[test]
fn a_failed_upgrade_is_the_error_and_the_audit_says_failed() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).unwrap();
    let asked = now(s.flow.ask(&preview)).unwrap();
    s.helm.script().history.push_ok(vec![revision(3, "15.0.0")]);
    s.helm
        .script()
        .upgrade
        .push_err(oxikube_domain::OxiError::internal(
            "UPGRADE FAILED: timed out waiting",
        ));
    let error = now(s.flow.start(&preview, &asked).run).unwrap_err();
    assert!(
        matches!(&error, UpgradeError::Failed(m) if m.contains("timed out")),
        "{error:?}"
    );
    assert_eq!(
        s.audit_of_upgrades().last().unwrap().outcome,
        AuditOutcome::Failed
    );
}

#[test]
fn declining_upgrades_nothing_and_is_audited_as_cancelled() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).unwrap();
    let asked = now(s.flow.ask(&preview)).unwrap();
    now(s.flow.decline(asked));
    assert!(s.upgrades().iter().all(|r| r.dry_run));
    assert_eq!(
        s.audit_of_upgrades().last().unwrap().outcome,
        AuditOutcome::Cancelled
    );
}

#[test]
fn cancel_kills_the_stream_and_is_audited() {
    let s = Setup::new(false);
    s.script_preview();
    let preview = now(s.flow.preview(spec())).unwrap();
    let asked = now(s.flow.ask(&preview)).unwrap();
    s.helm.hold_streams(true);
    s.helm.script().history.push_ok(vec![revision(3, "15.0.0")]);
    s.helm
        .script()
        .upgrade
        .push_ok(vec![line("waiting for resources")]);
    let mut start = s.flow.start(&preview, &asked);
    assert!(
        (&mut start.run).now_or_never().is_none(),
        "helm is still running"
    );
    assert_eq!(s.helm.open_streams(), 1);
    drop(start.run);
    assert_eq!(s.helm.open_streams(), 0);
    assert!(matches!(
        start.lines.try_recv(),
        Ok(_) | Err(futures::channel::mpsc::TryRecvError::Closed)
    ));
    s.helm.hold_streams(false);
    s.script_preview();
    now(s.flow.preview(spec())).unwrap();
    let outcomes: Vec<_> = s.audit_of_upgrades().iter().map(|r| r.outcome).collect();
    assert!(outcomes.contains(&AuditOutcome::Cancelled), "{outcomes:?}");
}

#[test]
fn read_only_blocks_the_preview_and_the_upgrade() {
    let s = Setup::new(true);
    let error = now(s.flow.preview(spec())).unwrap_err();
    assert!(matches!(error, UpgradeError::ReadOnly { .. }), "{error:?}");
    assert!(s.helm.recorded_calls().is_empty());
    let outcomes: Vec<_> = s.audit_of_upgrades().iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [AuditOutcome::Denied]);
}

#[test]
fn the_current_release_is_read_natively_with_its_user_values() {
    let s = Setup::new(false);
    let mut failed = revision(4, "15.1.0");
    failed.status = ReleaseStatus::Failed;
    s.helm
        .script()
        .history
        .push_ok(vec![failed, revision(3, "15.0.0")]);
    s.helm.script().values.push_ok(values(
        "replicaCount: 2\nauth:\n  password: hunter2-value\n",
    ));
    let current = now(s.flow.current(&web())).expect("read");
    assert_eq!(current.chart.name, "nginx");
    assert_eq!(current.chart.version, "15.1.0");
    assert_eq!(current.revision, 4);
    assert_eq!(current.status, ReleaseStatus::Failed);
    assert!(current.values.expose().contains("replicaCount: 2"));
    assert!(!format!("{current:?}").contains("hunter2"));
    // Only reads: nothing mutating, no binary needed.
    assert!(s.helm.recorded_calls().iter().all(|c| !c.is_mutating()));

    s.helm.script().history.push_ok(vec![]);
    s.helm.script().values.push_ok(values(""));
    assert!(now(s.flow.current(&web())).is_err());
}

#[test]
fn a_release_with_no_values_has_empty_text() {
    let s = Setup::new(false);
    s.helm.script().history.push_ok(vec![revision(1, "1.0.0")]);
    s.helm.script().values.push_ok(values(""));
    let current = now(s.flow.current(&web())).unwrap();
    assert!(current.values.is_blank());
}
