//! Validated field types of the per-cluster settings.
//!
//! Each type deserialises from a string and refuses a bad value with a message that says what
//! to write instead (it never echoes the value: a mistyped URL may carry a credential). A
//! refused value is a type error of the cluster's block, so the store keeps the last good
//! block and records a diagnostic.

use oxikube_ports::SecretKey;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// Keychain namespace of the secrets the cluster settings refer to.
const PROMETHEUS_SECRET_NAMESPACE: &str = "prometheus";

/// An `http(s)` URL without credentials, query or fragment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HttpUrl(String);

impl HttpUrl {
    /// The URL text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HttpUrl {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let text = value.trim();
        let rest = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))
            .ok_or("a URL must start with http:// or https://")?;
        if rest.is_empty() || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("a URL must be a single http(s) address".into());
        }
        if text.contains(['?', '#']) {
            return Err(
                "a URL must not carry a query or fragment: keep tokens out of settings \
                 and use `auth_secret` to name a keychain entry"
                    .into(),
            );
        }
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.contains('@') {
            return Err(
                "a URL must not contain credentials: keep tokens out of settings \
                 and use `auth_secret` to name a keychain entry"
                    .into(),
            );
        }
        Ok(Self(text.to_owned()))
    }
}

impl From<HttpUrl> for String {
    fn from(value: HttpUrl) -> Self {
        value.0
    }
}

impl JsonSchema for HttpUrl {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "HttpUrl".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^https?://[^\\s@?#/]+(/[^\\s?#]*)?$",
            "description": "An http(s) URL without credentials, query or fragment.",
            "examples": ["https://prometheus.example.com:9090"]
        })
    }
}

/// The name of a keychain entry (namespace `prometheus`) that holds a token. The settings file
/// stores this reference only, never the token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretName(String);

impl SecretName {
    /// The keychain key this name refers to.
    pub fn key(&self) -> SecretKey {
        // Validated on construction, so this cannot fail.
        SecretKey::new(PROMETHEUS_SECRET_NAMESPACE, &self.0)
            .unwrap_or_else(|_| unreachable!("validated by SecretName::try_from"))
    }
}

impl TryFrom<String> for SecretName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let name = value.trim();
        SecretKey::new(PROMETHEUS_SECRET_NAMESPACE, name).map_err(|_| {
            "a secret name must be non-empty and contain no `/` or control characters; it names \
             a keychain entry, it is not the secret itself"
                .to_owned()
        })?;
        Ok(Self(name.to_owned()))
    }
}

impl From<SecretName> for String {
    fn from(value: SecretName) -> Self {
        value.0
    }
}

impl JsonSchema for SecretName {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SecretName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "pattern": "^[^/\\p{Cc}]+$",
            "description": "The name of a keychain entry holding the token. Never the token itself.",
            "examples": ["prod-prometheus"]
        })
    }
}

/// Schema of `colour`: `#rrggbb` or `#rgb`, or `null` to use the default.
pub(super) fn colour_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": ["string", "null"],
        "pattern": "^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$",
        "examples": ["#e5484d"]
    })
}

/// Schema of `exec_interactivity`: the snake_case variants of `ExecInteractivity`.
pub(super) fn exec_interactivity_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": ["string", "null"],
        "enum": ["never", "if_available", "always", null]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Result<HttpUrl, String> {
        HttpUrl::try_from(text.to_owned())
    }

    #[test]
    fn urls_must_be_plain_http() {
        assert!(url("https://prom.example.com:9090/prefix").is_ok());
        assert!(url("http://10.0.0.1:9090").is_ok());
        assert!(url("prom.example.com").is_err());
        assert!(url("ftp://prom").is_err());
        assert!(url("https://").is_err());
        assert!(url("https://prom example").is_err());
    }

    #[test]
    fn urls_refuse_credentials_without_echoing_them() {
        for bad in [
            "https://user:hunter2@prom.example.com",
            "https://prom.example.com/?token=hunter2",
            "https://prom.example.com/#hunter2",
        ] {
            let err = url(bad).unwrap_err();
            assert!(!err.contains("hunter2"), "{err}");
            assert!(err.contains("auth_secret"), "{err}");
        }
        // `@` in the path is fine; only the authority carries userinfo.
        assert!(url("https://prom.example.com/a@b").is_ok());
    }

    #[test]
    fn secret_names_refuse_slashes_and_blanks() {
        assert!(SecretName::try_from("prod-prometheus".to_owned()).is_ok());
        assert!(SecretName::try_from(" ".to_owned()).is_err());
        assert!(SecretName::try_from("a/b".to_owned()).is_err());
        let name = SecretName::try_from(" prod ".to_owned()).unwrap();
        assert_eq!(name.key().to_string(), "prometheus/prod");
    }
}
