//! Getting a token for a ServiceAccount: `TokenRequest`, or on a cluster without it a legacy
//! `kubernetes.io/service-account-token` Secret.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::ids::Gvk;
use oxikube_domain::secret::SecretText;
use oxikube_domain::{ErrorKind, OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourceReader, ResourceWriter, WriteOptions};
use serde_json::{Value, json};

/// The lifetime asked for when the caller names none: one hour.
pub const DEFAULT_EXPIRATION_SECONDS: u32 = 3600;

/// The shortest lifetime the API server accepts for a `TokenRequest`.
pub const MIN_EXPIRATION_SECONDS: u32 = 600;

/// The Secret type of a legacy ServiceAccount token.
const LEGACY_TYPE: &str = "kubernetes.io/service-account-token";

/// The annotation that ties a legacy token Secret to its account.
const ACCOUNT_ANNOTATION: &str = "kubernetes.io/service-account.name";

/// How many times a new legacy token Secret is read while the token controller fills it in.
const FILL_ATTEMPTS: u32 = 15;

/// Time between those reads.
const FILL_INTERVAL: Duration = Duration::from_millis(300);

/// A token and when the server says it expires.
pub(super) struct IssuedToken {
    pub(super) token: SecretText,
    pub(super) expires_at: Option<String>,
}

/// The `TokenRequest` body for a lifetime and audiences. No credential is in it.
pub(super) fn token_request_body(expiration_seconds: u32, audiences: &[String]) -> Value {
    let mut spec = json!({"expirationSeconds": expiration_seconds});
    if !audiences.is_empty() {
        spec["audiences"] = json!(audiences);
    }
    json!({
        "apiVersion": "authentication.k8s.io/v1",
        "kind": "TokenRequest",
        "spec": spec,
    })
}

/// The lifetime to ask for.
///
/// # Errors
///
/// A validation error for less than [`MIN_EXPIRATION_SECONDS`], which the server would refuse.
pub(super) fn checked_expiration(requested: Option<u32>) -> OxiResult<u32> {
    let seconds = requested.unwrap_or(DEFAULT_EXPIRATION_SECONDS);
    if seconds < MIN_EXPIRATION_SECONDS {
        return Err(OxiError::validation(format!(
            "a token lives at least {MIN_EXPIRATION_SECONDS} seconds (10 minutes); {seconds} is too short"
        )));
    }
    Ok(seconds)
}

/// The token and expiry in a `TokenRequest` response.
///
/// # Errors
///
/// An internal error (with no part of the response in the message) for a response with no token.
pub(super) fn parse_issued(response: &Value) -> OxiResult<IssuedToken> {
    let status = response.get("status");
    let token = status
        .and_then(|status| status.get("token"))
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| OxiError::internal("the API server returned no token"))?;
    let expires_at = status
        .and_then(|status| status.get("expirationTimestamp"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(IssuedToken {
        token: SecretText::new(token),
        expires_at,
    })
}

/// Whether `error` from a `TokenRequest` means the cluster has no such API (older than 1.22, or
/// the feature disabled), as opposed to a refusal such as RBAC. The account itself was read just
/// before, so a not-found here is the subresource.
pub(super) fn lacks_token_request(error: &OxiError) -> bool {
    matches!(error.kind(), ErrorKind::NotFound | ErrorKind::Unsupported)
}

fn secret_gvk() -> Gvk {
    Gvk::new("", "v1", "Secret")
}

/// The token of an existing legacy Secret of `account`, if the namespace has one.
///
/// # Errors
///
/// The list's error (`Forbidden` when the user may not list Secrets).
pub(super) async fn existing_legacy_token(
    reader: &dyn ResourceReader,
    namespace: &str,
    account: &str,
) -> OxiResult<Option<SecretText>> {
    let options = ListOptions::default().fields(format!("type={LEGACY_TYPE}"));
    let page = reader
        .list(&secret_gvk(), Some(namespace), &options)
        .await?;
    for secret in &page.items {
        if secret
            .meta
            .annotations
            .get(ACCOUNT_ANNOTATION)
            .map(|name| &**name)
            != Some(account)
        {
            continue;
        }
        if let Some(token) = decoded_token(secret) {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

/// The manifest of a legacy token Secret for `account`.
pub(super) fn legacy_secret_manifest(namespace: &str, account: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "type": LEGACY_TYPE,
        "metadata": {
            "generateName": format!("{account}-token-"),
            "namespace": namespace,
            "annotations": {ACCOUNT_ANNOTATION: account},
        },
    })
}

/// Creates a legacy token Secret for `account` and waits for the token controller to fill it.
///
/// # Errors
///
/// The create's error, or a timeout when the controller does not answer in time.
pub(super) async fn create_legacy_token(
    reader: &dyn ResourceReader,
    writer: &dyn ResourceWriter,
    namespace: &str,
    account: &str,
    options: &WriteOptions,
) -> OxiResult<SecretText> {
    let manifest = legacy_secret_manifest(namespace, account);
    let created = writer
        .create(&secret_gvk(), Some(namespace), &manifest, options)
        .await?;
    let name = created.name().to_owned();
    for _ in 0..FILL_ATTEMPTS {
        let secret = reader.get(&secret_gvk(), Some(namespace), &name).await?;
        if let Some(token) = decoded_token(&secret) {
            return Ok(token);
        }
        tokio::time::sleep(FILL_INTERVAL).await;
    }
    Err(OxiError::new(
        ErrorKind::Timeout,
        format!("the token controller did not fill Secret {namespace}/{name} in time"),
    ))
}

/// The `data.token` of a legacy token Secret, decoded.
fn decoded_token(secret: &Resource) -> Option<SecretText> {
    let encoded = secret.get_str("/data/token")?;
    let bytes = STANDARD.decode(encoded.trim()).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    (!text.is_empty()).then(|| SecretText::new(text))
}
