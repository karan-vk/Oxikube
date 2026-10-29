//! The audit trail of a copy: one record on the target for the mutation and one on the source
//! for the read, linked by `copied_from` / `copied_to`, never carrying a value.

use oxikube_domain::audit::AuditOutcome;

use super::*;

fn secret() -> Value {
    json!({"apiVersion": "v1", "kind": "Secret", "type": "Opaque",
        "metadata": {"name": "db", "namespace": "shop"},
        "data": {"password": "aHVudGVyMi12ZXJ5LXNlY3JldA=="},
        "stringData": {"token": "tok-plaintext-9"}})
}

#[test]
fn both_clusters_get_a_record() {
    let t = Copying::new(false);
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    t.copy(&preview, None).expect("copy");

    let real: Vec<_> =
        t.h.audit()
            .into_iter()
            .filter(|r| r.cmd.as_ref() == "resource::CopyTo" && !r.dry_run)
            .collect();
    assert_eq!(real.len(), 2, "{real:?}");
    let on_target = real
        .iter()
        .find(|r| r.cluster == id("b"))
        .expect("target record");
    let on_source = real
        .iter()
        .find(|r| r.cluster == id("a"))
        .expect("source record");

    assert_eq!(on_target.outcome, AuditOutcome::Succeeded);
    assert_eq!(on_target.target.namespace(), Some("prod"));
    let detail = on_target.detail.as_deref().unwrap();
    assert!(
        detail.contains("copied_from=ConfigMap shop/settings"),
        "{detail}"
    );
    assert!(
        detail.contains(&format!("source_cluster={}", id("a"))),
        "{detail}"
    );
    assert!(detail.contains("source_context=a"), "{detail}");

    assert_eq!(on_source.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        on_source.target.namespace(),
        Some("shop"),
        "the record names the source object"
    );
    let detail = on_source.detail.as_deref().unwrap();
    assert!(
        detail.contains("copied_to=ConfigMap prod/settings"),
        "{detail}"
    );
    assert!(
        detail.contains(&format!("target_cluster={}", id("b"))),
        "{detail}"
    );
}

#[test]
fn the_dry_run_is_audited_on_both_clusters_as_a_dry_run() {
    let t = Copying::new(false);
    t.preview(t.request(&configmap(), "prod")).expect("preview");
    let dry: Vec<_> = t.h.audit().into_iter().filter(|r| r.dry_run).collect();
    assert_eq!(dry.len(), 2);
    assert!(dry.iter().any(|r| r.cluster == id("a")));
    assert!(dry.iter().any(|r| r.cluster == id("b")));
}

#[test]
fn a_failed_source_read_is_recorded_as_failed_on_the_source() {
    let t = Copying::new(false);
    let request = CopyRequest::to(source_ref(&configmap()), id("b"), Some("prod"));
    t.preview(request).unwrap_err();
    let on_source =
        t.h.audit()
            .into_iter()
            .find(|r| r.cluster == id("a"))
            .expect("a record on the source");
    assert_eq!(on_source.outcome, AuditOutcome::Failed);
}

#[test]
fn no_secret_value_reaches_the_audit_log_the_preview_or_the_error_text() {
    let t = Copying::new(false);
    let preview = t.preview(t.request(&secret(), "prod")).expect("preview");
    t.copy(&preview, None).expect("copy");
    let everything = format!(
        "{:?} {:?} {}",
        t.h.audit(),
        preview.removed,
        serde_json::to_string(&t.h.audit()).unwrap()
    );
    for value in ["aHVudGVy", "hunter2", "tok-plaintext-9"] {
        assert!(!everything.contains(value), "{value} leaked: {everything}");
    }
    assert!(!preview.result.to_string().contains("tok-plaintext-9"));
    assert!(!format!("{preview:?}").contains("aHVudGVy"));
}

#[test]
fn a_denied_copy_still_says_where_it_came_from() {
    let t = Copying::new(true);
    t.preview(t.request(&configmap(), "prod")).unwrap_err();
    let record = t.h.audit().into_iter().next().expect("a record");
    assert_eq!(record.outcome, AuditOutcome::Denied);
    assert!(
        record
            .detail
            .as_deref()
            .unwrap()
            .contains("copied_from=ConfigMap shop/settings")
    );
}
