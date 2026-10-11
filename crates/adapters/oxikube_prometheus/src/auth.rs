//! Resolving an endpoint's [`PromAuth`] into the request's credentials.
//!
//! The token or password is read from memory or the keychain at request time, held only for the
//! duration of building the request, and never formatted into a log or an error.

use std::sync::Arc;

use oxikube_ports::secrets::ExposeSecret;
use oxikube_ports::{PromAuth, PromSecret, PromqlError, SecretStorePort};
use reqwest::RequestBuilder;

/// Adds the `Authorization` header for `auth` to `request` (nothing for [`PromAuth::None`]).
pub(crate) async fn apply(
    request: RequestBuilder,
    auth: &PromAuth,
    secrets: Option<&Arc<dyn SecretStorePort>>,
) -> Result<RequestBuilder, PromqlError> {
    match auth {
        PromAuth::None => Ok(request),
        PromAuth::Bearer(secret) => {
            let token = resolve(secret, secrets).await?;
            Ok(request.bearer_auth(token))
        }
        PromAuth::Basic { username, password } => {
            let password = resolve(password, secrets).await?;
            Ok(request.basic_auth(username, Some(password)))
        }
    }
}

/// The secret's value, trimmed (a token pasted or stored with a trailing newline is common).
async fn resolve(
    secret: &PromSecret,
    store: Option<&Arc<dyn SecretStorePort>>,
) -> Result<String, PromqlError> {
    let value = match secret {
        PromSecret::Memory(value) => value.expose_secret().trim().to_owned(),
        PromSecret::Keychain(key) => {
            let store = store.ok_or_else(|| {
                PromqlError::Unauthorized(format!(
                    "credential `{key}` is in the keychain but this client has no keychain access"
                ))
            })?;
            match store.get(key).await {
                Ok(Some(value)) => value.expose_secret().trim().to_owned(),
                Ok(None) => {
                    return Err(PromqlError::Unauthorized(format!(
                        "no credential is stored under `{key}`"
                    )));
                }
                Err(error) => {
                    return Err(PromqlError::Unauthorized(format!(
                        "cannot read credential `{key}` from the keychain ({})",
                        error.kind()
                    )));
                }
            }
        }
    };
    if value.is_empty() {
        return Err(PromqlError::Unauthorized("the credential is empty".into()));
    }
    Ok(value)
}
