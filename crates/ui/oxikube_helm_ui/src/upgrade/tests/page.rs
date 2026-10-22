//! The upgrade tab in a window: prefilled values, the version picker, the diffs, `--reuse-values`,
//! the preview, the medium confirmation, the stream, cancel.

use gpui::TestAppContext;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::helm::{OutputLine, ReleaseStatus};
use oxikube_testkit::HelmCall;

use super::{Harness, Options, revision, web};
use crate::upgrade::{HelmUpgradeEvent, Pane, Stage};

fn upgrades(h: &Harness) -> Vec<oxikube_domain::helm::UpgradeRequest> {
    h.fake
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            HelmCall::Upgrade(r) => Some(r),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn the_tab_opens_with_the_releases_values_chart_versions_and_defaults(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Editing);
    // The release's current user values are in the editor (read natively).
    assert_eq!(h.read(|v, cx| v.values(cx)).expose(), super::CURRENT_VALUES);
    let current = h.read(|v, _| v.current().cloned()).expect("read");
    assert_eq!(current.chart.to_string(), "nginx-15.0.0");
    assert_eq!(current.revision, 3);
    // The chart source defaults to the repository that has the running version.
    let form = h.read(|v, cx| v.form(cx));
    assert_eq!(form.chart, "bitnami/nginx");
    assert_eq!(form.version, "", "the newest unless another is picked");
    assert_eq!(h.read(|v, _| v.sources().len()), 1);
    assert_eq!(
        h.read(|v, _| v.sources()[0].versions.clone()),
        ["15.4.0", "15.3.1", "15.0.0"]
    );
    for selector in [
        "helm-upgrade-chart",
        "helm-upgrade-sources",
        "helm-upgrade-versions",
        "helm-upgrade-reuse",
        "helm-upgrade-values",
        "helm-upgrade-preview",
        "helm-upgrade-upgrade",
        "helm-upgrade-review",
        "helm-upgrade-current",
    ] {
        assert!(h.is_laid_out(selector), "{selector}");
    }
    // Reading the release changes nothing and needs no mutation.
    assert!(h.fake.recorded_calls().iter().all(|c| !c.is_mutating()));
}

#[gpui::test]
fn the_values_diff_follows_the_edits_and_never_shows_a_secret(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    assert!(h.read(|v, _| v.values_diff().is_some_and(|d| d.is_empty())));
    h.set_values("replicaCount: 3\nimage:\n  tag: new\nauth:\n  password: hunter3-value\n");
    let diff = h.read(|v, _| v.values_diff().cloned()).expect("computed");
    assert!(!diff.is_empty());
    assert_eq!(diff.counts(), (3, 3));
    let text: String = (0..64u32)
        .map(|i| format!("{}\n{}\n", diff.old_line(i), diff.new_line(i)))
        .collect();
    assert!(text.contains("tag: new"));
    assert!(!text.contains("hunter"), "{text}");
    assert!(h.is_laid_out("helm-upgrade-diff-values-summary"));
}

#[gpui::test]
fn the_chart_defaults_diff_shows_what_the_new_version_adds_and_renames(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    let diff = h
        .read(|v, _| v.defaults_diff().cloned())
        .expect("both read");
    assert_eq!(diff.counts(), (1, 3));
    h.click("helm-upgrade-tab-defaults");
    assert_eq!(h.read(|v, _| v.pane()), Pane::ChartDefaults);
    assert!(h.is_laid_out("helm-upgrade-diff-defaults"));
}

#[gpui::test]
fn picking_a_version_sends_it_and_reads_that_versions_defaults(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.fake.script().show.push_ok(super::DEFAULTS_OLD.to_owned());
    h.click("helm-upgrade-version:2");
    assert_eq!(h.read(|v, cx| v.form(cx).version), "15.3.1");
    // 15.3.1 has the same defaults as the running version: no key changed.
    assert!(h.read(|v, _| v.defaults_diff().is_some_and(|d| d.is_empty())));
    h.script_preview();
    h.click("helm-upgrade-preview");
    let calls = h.fake.recorded_calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, HelmCall::Template(r) if r.chart.version == "15.3.1"))
    );
}

#[gpui::test]
fn preview_lists_the_objects_it_adds_changes_and_removes_then_upgrade_asks_medium_and_streams(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::open(cx);
    h.set_values("replicaCount: 3\nauth:\n  password: hunter2-value\n");
    h.script_preview();
    h.click("helm-upgrade-preview");

    assert_eq!(h.read(|v, _| v.pane()), Pane::Preview);
    let manifest = h.read(|v, cx| v.manifest_text(cx));
    assert!(manifest.contains("kind: Ingress"), "{manifest}");
    for selector in [
        "helm-upgrade-changes-added",
        "helm-upgrade-changes-changed",
        "helm-upgrade-changes-removed",
        "helm-upgrade-manifest",
    ] {
        assert!(h.is_laid_out(selector), "{selector}");
    }
    let shown = h.read(|v, cx| v.fresh_preview(cx).cloned()).expect("fresh");
    assert_eq!(shown.preview.from_chart, "nginx-15.0.0");
    assert_eq!(shown.preview.changes.added.len(), 1);
    assert_eq!(shown.preview.changes.changed.len(), 1);
    assert_eq!(shown.preview.changes.removed.len(), 1);

    h.click("helm-upgrade-upgrade");
    let Stage::Confirming(request) = h.read(|v, _| v.stage().clone()) else {
        panic!("the guard asks first");
    };
    assert_eq!(request.risk, Some(oxikube_domain::safety::Risk::Medium));
    assert!(
        request.summary.contains("bitnami/nginx latest"),
        "{}",
        request.summary
    );
    assert!(h.is_laid_out("helm-upgrade-confirm"));
    // Only the dry run so far.
    assert!(upgrades(&h).iter().all(|r| r.dry_run));

    h.fake
        .script()
        .history
        .push_ok(vec![revision(3, "15.0.0", ReleaseStatus::Deployed)]);
    h.fake.script().upgrade.push_ok(vec![
        OutputLine::stdout("Release \"web\" has been upgraded."),
        OutputLine::stderr("warning: deprecated"),
        OutputLine::stdout("REVISION: 4"),
    ]);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    let view = h.view.clone();
    let _subscription = h.vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &HelmUpgradeEvent, _| {
            sink.borrow_mut().push(event.clone());
        })
    });
    h.click("helm-upgrade-confirm-yes");
    assert_eq!(
        h.read(|v, _| v.stage().clone()),
        Stage::Upgraded("Upgraded release web in namespace demo".into())
    );
    assert_eq!(*events.borrow(), [HelmUpgradeEvent::Upgraded(web())]);
    assert_eq!(h.read(|v, _| v.upgraded().cloned()), Some(web()));
    assert_eq!(
        h.read(|v, _| v.log().text()),
        "Release \"web\" has been upgraded.\n! warning: deprecated\nREVISION: 4\n"
    );
    assert!(h.is_laid_out("helm-upgrade-log"));
    let real = upgrades(&h).pop().unwrap();
    assert!(!real.dry_run && !real.reuse_values);
    assert_eq!(real.values.get("replicaCount"), Some(&serde_json::json!(3)));
    // Audited: the dry run and the upgrade, with the chart it ran, never the values.
    let audit = h.audit();
    assert_eq!(audit.len(), 2);
    assert!(audit[0].dry_run && !audit[1].dry_run);
    assert_eq!(audit[1].outcome, AuditOutcome::Succeeded);
    let detail = audit[1].detail.clone().unwrap();
    assert!(
        detail.contains("reuse_values=false") && detail.contains("from=nginx-15.0.0"),
        "{detail}"
    );
    assert!(!format!("{audit:?}").contains("hunter2"));
}

#[gpui::test]
fn reuse_values_is_explained_and_sent(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    assert!(
        !h.read(|v, cx| v.form(cx).reuse_values),
        "off unless chosen"
    );
    h.click("helm-upgrade-reuse");
    assert!(h.read(|v, cx| v.form(cx).reuse_values));
    assert!(h.is_laid_out("helm-upgrade-reuse-help"));
    h.fake
        .script()
        .values
        .push_ok(oxikube_domain::helm::Values::from_yaml("old: kept\n").unwrap());
    h.script_preview();
    h.click("helm-upgrade-preview");
    let calls = h.fake.recorded_calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, HelmCall::Values { all: true, .. }))
    );
    assert!(upgrades(&h).iter().all(|r| r.reuse_values && r.dry_run));
    h.click("helm-upgrade-upgrade");
    let Stage::Confirming(request) = h.read(|v, _| v.stage().clone()) else {
        panic!("asked");
    };
    assert!(request.summary.contains("reusing the previous values"));
}

#[gpui::test]
fn an_edit_after_the_preview_makes_it_stale(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.script_preview();
    h.click("helm-upgrade-preview");
    assert!(h.read(|v, cx| v.fresh_preview(cx).is_some()));
    h.set_values("replicaCount: 9\n");
    assert!(h.read(|v, cx| v.fresh_preview(cx).is_none()));
    h.update(|view, _, cx| view.upgrade(cx));
    assert_eq!(
        h.read(|v, _| v.stage().clone()),
        Stage::Editing,
        "needs a fresh preview"
    );
}

#[gpui::test]
fn a_server_refusal_is_shown_inline_and_blocks_the_upgrade(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    let script = h.fake.script();
    script.manifest.push_ok(super::CURRENT_MANIFEST.to_owned());
    script
        .history
        .push_ok(vec![revision(3, "15.0.0", ReleaseStatus::Deployed)]);
    script.template.push_ok(super::RENDERED.to_owned());
    script.upgrade.push_err(oxikube_domain::OxiError::internal(
        "another operation (install/upgrade/rollback) is in progress",
    ));
    h.click("helm-upgrade-preview");
    assert!(h.is_laid_out("helm-upgrade-server-error"));
    h.update(|view, _, cx| view.upgrade(cx));
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Editing);
}

#[gpui::test]
fn a_template_error_is_shown_inline(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    let script = h.fake.script();
    script.manifest.push_ok(super::CURRENT_MANIFEST.to_owned());
    script
        .history
        .push_ok(vec![revision(3, "15.0.0", ReleaseStatus::Deployed)]);
    script
        .template
        .push_err(oxikube_domain::OxiError::validation(
            "values don't meet the schema",
        ));
    script.upgrade.push_ok(Vec::new());
    h.click("helm-upgrade-preview");
    assert!(h.is_laid_out("helm-upgrade-preview-error"));
    assert!(h.read(|v, _| v.preview_error().unwrap().contains("schema")));
}

#[gpui::test]
fn a_failed_upgrade_points_to_history_and_rollback(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.script_preview();
    h.click("helm-upgrade-preview");
    h.click("helm-upgrade-upgrade");
    h.fake
        .script()
        .history
        .push_ok(vec![revision(3, "15.0.0", ReleaseStatus::Deployed)]);
    h.fake
        .script()
        .upgrade
        .push_err(oxikube_domain::OxiError::internal(
            "UPGRADE FAILED: timed out waiting",
        ));
    h.click("helm-upgrade-confirm-yes");
    let Stage::Failed(why) = h.read(|v, _| v.stage().clone()) else {
        panic!("failed");
    };
    assert!(why.contains("timed out"), "{why}");
    assert!(
        why.contains("History") && why.contains("roll back"),
        "{why}"
    );
    assert!(h.read(|v, _| v.upgraded().is_none()));
}

#[gpui::test]
fn cancel_stops_helm(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.script_preview();
    h.click("helm-upgrade-preview");
    h.click("helm-upgrade-upgrade");
    h.fake.hold_streams(true);
    h.fake
        .script()
        .history
        .push_ok(vec![revision(3, "15.0.0", ReleaseStatus::Deployed)]);
    h.fake
        .script()
        .upgrade
        .push_ok(vec![OutputLine::stdout("waiting for resources")]);
    h.click("helm-upgrade-confirm-yes");
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Upgrading);
    assert_eq!(h.fake.open_streams(), 1);
    assert_eq!(h.read(|v, _| v.log().text()), "waiting for resources\n");
    h.click("helm-upgrade-cancel");
    assert_eq!(h.fake.open_streams(), 0, "the helm process is killed");
    assert!(
        matches!(h.read(|v, _| v.stage().clone()), Stage::Failed(why) if why.starts_with("Cancelled"))
    );
    assert!(h.read(|v, _| v.upgraded().is_none()));
}

#[gpui::test]
fn declining_the_confirmation_returns_to_the_form(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx);
    h.script_preview();
    h.click("helm-upgrade-preview");
    h.click("helm-upgrade-upgrade");
    h.click("helm-upgrade-confirm-no");
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
    h.click("helm-upgrade-preview");
    let error = h
        .read(|v, _| v.preview_error().map(str::to_owned))
        .expect("refused");
    assert!(error.contains("read-only"), "{error}");
    assert!(
        !h.fake
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, HelmCall::Template(_) | HelmCall::Upgrade(_)))
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
    assert!(h.is_laid_out("helm-upgrade-guidance"));
    assert!(!h.is_laid_out("helm-upgrade-preview"));
    assert!(!h.is_laid_out("helm-upgrade-upgrade"));
    assert!(!h.is_laid_out("helm-upgrade-values"));
}

#[gpui::test]
fn without_a_release_the_tab_lists_the_clusters_releases_to_pick_from(cx: &mut TestAppContext) {
    let mut h = Harness::open_with(
        cx,
        Options {
            release: None,
            ..Options::default()
        },
    );
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Choosing);
    assert!(h.is_laid_out("helm-upgrade-picker"));
    // Sorted by namespace, then name: db, then web.
    assert!(h.is_laid_out("helm-upgrade-release:0"));
    h.click("helm-upgrade-release:1");
    assert_eq!(h.read(|v, _| v.target().cloned()), Some(web()));
    assert_eq!(h.read(|v, _| v.stage().clone()), Stage::Editing);
    assert!(h.is_laid_out("helm-upgrade-values"));
}
