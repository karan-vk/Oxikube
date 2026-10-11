//! Many documents in one apply: the order, the per-document results, a failure in the middle,
//! what waits for the namespace an earlier document makes, and the guard around all of it.

use oxikube_domain::OxiError;
use oxikube_domain::audit::AuditOutcome;
use oxikube_testkit::ResourceCall;

use super::*;
use crate::apply::{Change, DocumentStatus, PreviewOutcome};

const NAMESPACE: &str = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: shop\n";

fn configmap(name: &str, namespace: Option<&str>, mode: &str) -> String {
    let namespace = namespace.map_or_else(String::new, |ns| format!("  namespace: {ns}\n"));
    format!(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {name}\n{namespace}data:\n  mode: {mode}\n"
    )
}

fn live(name: &str, mode: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": name, "namespace": "default", "resourceVersion": "5"},
        "data": {"mode": mode}
    }))
    .expect("a valid object")
}

fn statuses(report: &crate::apply::BatchReport) -> Vec<&'static str> {
    report.documents.iter().map(|d| d.status.word()).collect()
}

/// The names of the objects written for real, in order.
fn applied_names(t: &Applying) -> Vec<String> {
    t.h.writes("a")
        .iter()
        .filter(|call| !call.is_dry_run())
        .map(|call| body(call)["metadata"]["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn documents_apply_in_buffer_order_each_with_its_own_dry_run_first() {
    let t = Applying::new(false);
    let (a, b) = (configmap("a", None, "x"), configmap("b", None, "y"));
    let preview = t.preview_batch(&[NAMESPACE, &a, &b]);
    assert_eq!(preview.documents.len(), 3);
    assert!(preview.documents.iter().all(|d| d.preview().is_some()));
    assert_eq!(preview.counts().create, 3);
    assert!(
        t.writes().iter().all(|(_, dry_run)| *dry_run),
        "the preview writes nothing"
    );
    assert_eq!(t.writes().len(), 3, "one dry run per document");

    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["created", "created", "created"]);
    assert_eq!(applied_names(&t), ["shop", "a", "b"], "in buffer order");
    assert_eq!(report.documents[0].label, "Namespace shop");
    assert_eq!(report.documents[1].label, "ConfigMap default/a");
    assert_eq!(report.summary(), "3 applied");
    assert_eq!(t.port().objects().len(), 3);
}

#[test]
fn each_document_says_created_configured_or_unchanged() {
    let t = Applying::new(false);
    t.port().insert(live("same", "x"));
    t.port().insert(live("changed", "old"));
    let docs = [
        configmap("new", None, "x"),
        configmap("same", None, "x"),
        configmap("changed", None, "new"),
    ];
    let preview = t.preview_batch(&[&docs[0], &docs[1], &docs[2]]);
    let changes: Vec<_> = preview.documents.iter().map(|d| d.change()).collect();
    assert_eq!(
        changes,
        [
            Some(Change::Create),
            Some(Change::Unchanged),
            Some(Change::Update)
        ]
    );
    let counts = preview.counts();
    assert_eq!((counts.create, counts.unchanged, counts.update), (1, 1, 1));
    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["created", "unchanged", "configured"]);
}

#[test]
fn a_failed_dry_run_is_reported_and_not_sent_while_the_others_apply() {
    let t = Applying::new(false);
    let docs = [
        configmap("a", None, "x"),
        configmap("b", None, "y"),
        configmap("c", None, "z"),
    ];
    // The dry runs answer in document order: the second is refused.
    t.port().script().patch.push_ok(live("a", "x"));
    t.port()
        .script()
        .patch
        .push_err(OxiError::validation("ConfigMap \"b\" is invalid"));
    let preview = t.preview_batch(&[&docs[0], &docs[1], &docs[2]]);
    assert!(matches!(
        preview.documents[1].outcome,
        PreviewOutcome::Failed(_)
    ));
    assert_eq!(preview.counts().failed, 1);
    assert_eq!(preview.applying(), 2);

    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["created", "failed", "created"]);
    assert_eq!(report.documents[1].message, "ConfigMap \"b\" is invalid");
    assert_eq!(applied_names(&t), ["a", "c"], "the failed one was not sent");
    assert_eq!(report.failures(), 1);
    assert_eq!(report.summary(), "2 applied, 1 failed");
}

#[test]
fn a_failure_while_applying_does_not_stop_the_rest_but_skips_what_needs_it() {
    let t = Applying::new(false);
    let in_shop = configmap("in-shop", Some("shop"), "x");
    let elsewhere = configmap("elsewhere", None, "y");
    let preview = t.preview_batch(&[NAMESPACE, &in_shop, &elsewhere]);
    assert_eq!(preview.documents[1].depends_on, Some(0));
    assert_eq!(preview.documents[2].depends_on, None);

    // The namespace is refused as it is written.
    t.port()
        .script()
        .patch
        .push_err(OxiError::forbidden("namespaces is forbidden"));
    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["failed", "skipped", "created"]);
    assert!(report.documents[0].message.contains("forbidden"));
    assert_eq!(
        report.documents[1].message,
        "skipped: it needs Namespace shop (document 1), which did not apply"
    );
    assert_eq!(applied_names(&t), ["elsewhere"]);
    assert_eq!(report.summary(), "1 applied, 1 failed, 1 skipped");
}

#[test]
fn a_document_in_a_namespace_an_earlier_one_makes_waits_for_it() {
    let t = Applying::new(false);
    let in_shop = configmap("in-shop", Some("shop"), "x");
    // The server cannot rehearse an object of a namespace that does not exist yet.
    t.port().script().patch.push_ok(live("shop", "-"));
    t.port()
        .script()
        .patch
        .push_err(OxiError::not_found("namespaces \"shop\" not found"));
    let preview = t.preview_batch(&[NAMESPACE, &in_shop]);
    assert_eq!(preview.documents[1].outcome, PreviewOutcome::Deferred);
    assert_eq!(preview.counts().deferred, 1);
    assert_eq!(preview.applying(), 2);

    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["created", "created"]);
    assert_eq!(applied_names(&t), ["shop", "in-shop"]);
    assert_eq!(report.documents[1].label, "ConfigMap shop/in-shop");
}

#[test]
fn without_an_earlier_namespace_the_refusal_stands() {
    let t = Applying::new(false);
    let in_shop = configmap("in-shop", Some("shop"), "x");
    t.port()
        .script()
        .patch
        .push_err(OxiError::not_found("namespaces \"shop\" not found"));
    let preview = t.preview_batch(&[&in_shop]);
    assert!(matches!(
        preview.documents[0].outcome,
        PreviewOutcome::Failed(_)
    ));
    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["failed"]);
    assert!(t.writes().iter().all(|(_, dry_run)| *dry_run));
}

#[test]
fn a_document_needing_one_that_failed_its_dry_run_is_skipped_at_once() {
    let t = Applying::new(false);
    let in_shop = configmap("in-shop", Some("shop"), "x");
    t.port()
        .script()
        .patch
        .push_err(OxiError::forbidden("namespaces is forbidden"));
    let preview = t.preview_batch(&[NAMESPACE, &in_shop]);
    assert!(matches!(
        preview.documents[0].outcome,
        PreviewOutcome::Failed(_)
    ));
    assert_eq!(preview.documents[1].outcome, PreviewOutcome::Skipped);
    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["failed", "skipped"]);
    assert!(t.writes().iter().all(|(_, dry_run)| *dry_run));
}

#[test]
fn a_custom_resource_waits_for_the_definition_an_earlier_document_makes() {
    let t = Applying::new(false);
    let crd = "apiVersion: apiextensions.k8s.io/v1\nkind: CustomResourceDefinition\nmetadata:\n  name: widgets.example.com\nspec:\n  group: example.com\n  names:\n    kind: Widget\n";
    let widget = "apiVersion: example.com/v1\nkind: Widget\nmetadata:\n  name: w\n";
    t.port().script().patch.push_ok(live("crd", "-"));
    t.port()
        .script()
        .patch
        .push_err(OxiError::not_found("no matches for kind \"Widget\""));
    let preview = t.preview_batch(&[crd, widget]);
    assert_eq!(preview.documents[1].depends_on, Some(0));
    assert_eq!(preview.documents[1].outcome, PreviewOutcome::Deferred);
}

#[test]
fn a_read_only_cluster_refuses_every_document_and_sends_nothing() {
    let t = Applying::new(true);
    let (a, b) = (configmap("a", None, "x"), configmap("b", None, "y"));
    let preview = t.preview_batch(&[&a, &b]);
    assert!(preview.documents.iter().all(|d| matches!(
        &d.outcome,
        PreviewOutcome::Failed(ApplyError::ReadOnly { .. })
    )));
    assert_eq!(preview.applying(), 0);
    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["failed", "failed"]);
    assert!(t.calls().is_empty(), "not even a read: {:?}", t.calls());
    // The guard recorded each refusal.
    let audit = t.h.audit();
    assert_eq!(audit.len(), 2);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn read_only_turned_on_between_the_preview_and_the_apply_blocks_every_document() {
    let t = Applying::new(false);
    let (a, b) = (configmap("a", None, "x"), configmap("b", None, "y"));
    let preview = t.preview_batch(&[&a, &b]);
    t.h.manager.set_read_only(&id("a"), true).unwrap();
    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["failed", "failed"]);
    assert!(t.writes().iter().all(|(_, dry_run)| *dry_run));
    assert!(t.port().objects().is_empty());
}

#[test]
fn every_document_is_audited_on_its_own_without_its_values() {
    let t = Applying::new(false);
    let secret = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: db\nstringData:\n  password: hunter2-very-secret\n";
    let a = configmap("a", None, "x");
    let preview = t.preview_batch(&[&a, secret]);
    t.apply_batch(&preview);
    let audit = t.h.audit();
    // Two dry runs, then two applies (each one recorded once, whatever the handler rehearses).
    assert_eq!(audit.len(), 4);
    let targets: Vec<String> = audit.iter().map(|r| r.target.name.to_string()).collect();
    assert_eq!(targets, ["a", "db", "a", "db"]);
    let text = format!("{audit:?}");
    assert!(!text.contains("hunter2"), "{text}");
    // The previews hold the Secret masked.
    let shown = format!("{:?}", preview.documents[1].preview().unwrap().result);
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[test]
fn the_field_manager_of_each_request_is_used_for_every_document() {
    let t = Applying::new(false);
    let (a, b) = (configmap("a", None, "x"), configmap("b", None, "y"));
    let requests = [&a, &b]
        .map(|text| t.request(text).managed_by(Some("team-a".into())))
        .to_vec();
    let preview = t
        .service
        .preview_batch(requests)
        .now_or_never()
        .expect("answers at once");
    t.apply_batch(&preview);
    let managers: Vec<String> =
        t.h.writes("a")
            .iter()
            .filter_map(|call| match call {
                ResourceCall::Patch { options, .. } => options.field_manager.clone(),
                _ => None,
            })
            .collect();
    assert!(managers.iter().all(|m| m == "team-a"), "{managers:?}");
}

#[test]
fn an_unparseable_document_is_a_failure_with_a_number_not_a_crash() {
    let t = Applying::new(false);
    let a = configmap("a", None, "x");
    let preview = t.preview_batch(&[&a, "{not yaml"]);
    assert_eq!(preview.documents[1].label, "Document 2");
    assert!(matches!(
        preview.documents[1].outcome,
        PreviewOutcome::Failed(ApplyError::Invalid { .. })
    ));
    let report = t.apply_batch(&preview);
    assert_eq!(statuses(&report), ["created", "failed"]);
    assert!(matches!(
        report.documents[1].status,
        DocumentStatus::Failed(_)
    ));
}

#[test]
fn a_long_buffer_is_dry_run_a_few_at_a_time_and_keeps_its_order() {
    let t = Applying::new(false);
    let docs: Vec<String> = (0..30)
        .map(|i| configmap(&format!("cm-{i:02}"), None, "x"))
        .collect();
    let texts: Vec<&str> = docs.iter().map(String::as_str).collect();
    let preview = t.preview_batch(&texts);
    let labels: Vec<&str> = preview.documents.iter().map(|d| d.label.as_str()).collect();
    assert_eq!(labels[0], "ConfigMap default/cm-00");
    assert_eq!(labels[29], "ConfigMap default/cm-29");
    assert_eq!(preview.counts().create, 30);
    let report = t.apply_batch(&preview);
    assert_eq!(report.failures(), 0);
    let expected: Vec<String> = (0..30).map(|i| format!("cm-{i:02}")).collect();
    assert_eq!(applied_names(&t), expected, "in buffer order");
}
