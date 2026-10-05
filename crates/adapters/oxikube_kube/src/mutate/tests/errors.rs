//! Server rejections become domain errors with their detail.

use oxikube_domain::{ConflictReason, ErrorKind};
use oxikube_ports::{DeleteOptions, Patch, ResourceWriter, WriteOptions};
use serde_json::{Value, json};

use super::harness::*;
use crate::fake_api::status_body;

fn with_details(code: u16, reason: &str, message: &str, details: Value) -> Value {
    let mut body = status_body(code, reason, message);
    body["details"] = details;
    body
}

async fn apply_error(api: &crate::fake_api::FakeApi) -> oxikube_domain::OxiError {
    let object = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "a"}, "data": {"k": "2"}});
    resources(api)
        .patch(
            &configmap_gvk(),
            Some("default"),
            "a",
            &Patch::apply(object, "oxikube", false),
            &WriteOptions::default(),
        )
        .await
        .unwrap_err()
}

#[tokio::test]
async fn an_apply_conflict_names_the_fields_and_their_managers() {
    let api = server();
    api.reply(
        CONFIGMAP_A,
        409,
        with_details(
            409,
            "Conflict",
            "Apply failed with 2 conflicts: conflicts with \"alpha\": .data.k; conflicts with \"helm\" using v1: .data.j",
            json!({"name": "a", "kind": "ConfigMap", "causes": [
                {"reason": "FieldManagerConflict", "message": "conflict with \"alpha\"", "field": ".data.k"},
                {"reason": "FieldManagerConflict", "message": "conflict with \"helm\" using v1", "field": ".data.j"},
            ]}),
        ),
    );
    let err = apply_error(&api).await;
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(!err.is_retryable());
    let details = err.conflict_details().expect("conflict details");
    assert_eq!(details.reason, ConflictReason::FieldOwnership);
    assert_eq!(details.managers(), ["alpha", "helm"]);
    assert_eq!(details.causes[0].field, ".data.k");
    assert_eq!(details.causes[0].manager.as_deref(), Some("alpha"));
    assert_eq!(details.causes[1].field, ".data.j");
}

#[tokio::test]
async fn a_stale_replace_is_a_stale_version_conflict() {
    let api = server();
    api.reply(
        CONFIGMAP_A,
        409,
        with_details(
            409,
            "Conflict",
            "Operation cannot be fulfilled on configmaps \"a\": the object has been modified; please apply your changes to the latest version and try again",
            json!({"name": "a", "kind": "configmaps"}),
        ),
    );
    let err = resources(&api)
        .replace(
            &configmap_gvk(),
            Some("default"),
            "a",
            &manifest("a"),
            &no_options(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    let details = err.conflict_details().expect("conflict details");
    assert_eq!(details.reason, ConflictReason::StaleVersion);
    assert!(details.causes.is_empty());
}

#[tokio::test]
async fn creating_an_existing_object_is_an_already_exists_conflict() {
    let api = server();
    api.reply(
        CONFIGMAPS,
        409,
        status_body(409, "AlreadyExists", "configmaps \"a\" already exists"),
    );
    let err = resources(&api)
        .create(
            &configmap_gvk(),
            Some("default"),
            &manifest("a"),
            &no_options(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(
        err.conflict_details().map(|d| d.reason),
        Some(ConflictReason::AlreadyExists)
    );
}

#[tokio::test]
async fn a_422_carries_the_rejected_field_paths() {
    let api = server();
    api.reply(
        CONFIGMAPS,
        422,
        with_details(
            422,
            "Invalid",
            "ConfigMap \"A_b\" is invalid: metadata.name: Invalid value: \"A_b\": a lowercase RFC 1123 subdomain must consist of lower case alphanumeric characters",
            json!({"name": "A_b", "kind": "ConfigMap", "causes": [
                {"reason": "FieldValueInvalid", "message": "Invalid value: \"A_b\": a lowercase RFC 1123 subdomain", "field": "metadata.name"},
                {"reason": "FieldValueRequired", "message": "Required value", "field": "data"},
            ]}),
        ),
    );
    let err = resources(&api)
        .create(
            &configmap_gvk(),
            Some("default"),
            &manifest("A_b"),
            &no_options(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    let details = err.validation_details().expect("validation details");
    let fields: Vec<_> = details.causes.iter().map(|c| c.field.as_str()).collect();
    assert_eq!(fields, ["metadata.name", "data"]);
    assert_eq!(details.causes[0].reason, "FieldValueInvalid");
    assert!(details.causes[0].manager.is_none());
}

#[tokio::test]
async fn cause_text_is_redacted() {
    let api = server();
    api.reply(
        CONFIGMAPS,
        422,
        with_details(
            422,
            "Invalid",
            "invalid",
            json!({"causes": [{"reason": "FieldValueInvalid",
                "message": "Invalid value: \"Authorization: Bearer abcdef.ghijkl.mnopqr\"",
                "field": "data"}]}),
        ),
    );
    let err = resources(&api)
        .create(
            &configmap_gvk(),
            Some("default"),
            &manifest("a"),
            &no_options(),
        )
        .await
        .unwrap_err();
    let details = err.validation_details().expect("validation details");
    assert!(
        !details.causes[0].message.contains("abcdef.ghijkl"),
        "{:?}",
        details.causes[0]
    );
}

#[tokio::test]
async fn other_statuses_map_through_the_taxonomy() {
    let cases = [
        (403, "Forbidden", ErrorKind::Forbidden, false),
        (404, "NotFound", ErrorKind::NotFound, false),
        (415, "UnsupportedMediaType", ErrorKind::Unsupported, false),
        (429, "TooManyRequests", ErrorKind::Network, true),
        (500, "InternalError", ErrorKind::Internal, true),
        (503, "ServiceUnavailable", ErrorKind::Network, true),
    ];
    for (code, reason, kind, retryable) in cases {
        let api = server();
        api.reply(CONFIGMAP_A, code, status_body(code, reason, "boom"));
        let err = resources(&api)
            .delete(
                &configmap_gvk(),
                Some("default"),
                "a",
                &DeleteOptions::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), kind, "HTTP {code}");
        assert_eq!(err.is_retryable(), retryable, "HTTP {code}");
        assert!(err.conflict_details().is_none() && err.validation_details().is_none());
    }
}

#[tokio::test]
async fn strategic_merge_on_a_custom_resource_is_unsupported() {
    let api = server();
    api.reply(
        WIDGET_A,
        415,
        status_body(
            415,
            "UnsupportedMediaType",
            "the body of the request was in an unknown format - accepted media types include: application/json-patch+json, application/merge-patch+json, application/apply-patch+yaml",
        ),
    );
    let err = resources(&api)
        .patch(
            &widget_gvk(),
            Some("default"),
            "a",
            &Patch::strategic(json!({"spec": {"size": 4}})),
            &WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}
