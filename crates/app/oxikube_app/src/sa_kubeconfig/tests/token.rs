//! The `TokenRequest` body and response, and the lifetime check.

use oxikube_domain::{ErrorKind, OxiError};
use serde_json::json;

use super::{TOKEN, issued};
use crate::sa_kubeconfig::token::{
    DEFAULT_EXPIRATION_SECONDS, MIN_EXPIRATION_SECONDS, checked_expiration, lacks_token_request,
    parse_issued, token_request_body,
};

#[test]
fn the_body_asks_for_a_lifetime_and_optional_audiences() {
    assert_eq!(
        token_request_body(900, &[]),
        json!({
            "apiVersion": "authentication.k8s.io/v1", "kind": "TokenRequest",
            "spec": {"expirationSeconds": 900},
        })
    );
    let body = token_request_body(900, &["https://a".to_owned(), "b".to_owned()]);
    assert_eq!(body["spec"]["audiences"], json!(["https://a", "b"]));
}

#[test]
fn the_lifetime_defaults_to_an_hour_and_refuses_less_than_the_servers_minimum() {
    assert_eq!(
        checked_expiration(None).unwrap(),
        DEFAULT_EXPIRATION_SECONDS
    );
    assert_eq!(DEFAULT_EXPIRATION_SECONDS, 3600);
    assert_eq!(
        checked_expiration(Some(MIN_EXPIRATION_SECONDS)).unwrap(),
        MIN_EXPIRATION_SECONDS
    );
    let error = checked_expiration(Some(MIN_EXPIRATION_SECONDS - 1)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
}

#[test]
fn the_response_gives_the_token_and_the_expiry_the_server_chose() {
    let issued = parse_issued(&issued(TOKEN, "2026-10-28T13:00:00Z")).unwrap();
    assert_eq!(issued.token.expose(), TOKEN);
    assert_eq!(issued.expires_at.as_deref(), Some("2026-10-28T13:00:00Z"));
}

#[test]
fn a_response_without_a_token_is_an_error_that_quotes_nothing() {
    let error = parse_issued(&json!({"status": {"token": ""}, "secret": "x"}))
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(error.message(), "the API server returned no token");
}

#[test]
fn only_a_missing_api_means_no_token_request() {
    assert!(lacks_token_request(&OxiError::not_found(
        "serviceaccounts/token"
    )));
    assert!(lacks_token_request(&OxiError::unsupported("no")));
    assert!(!lacks_token_request(&OxiError::forbidden("rbac")));
    assert!(!lacks_token_request(&OxiError::validation("bad")));
}
