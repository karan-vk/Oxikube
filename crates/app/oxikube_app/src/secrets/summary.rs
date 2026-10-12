//! [`SecretSummary`]: what the Secret detail shows without anyone revealing a value.
//!
//! The summary is computed from the full object inside the read that fetches it, then the values
//! are dropped: it holds the key names and sizes, and the facts a type puts on display that are
//! not secret (the registries and usernames of a registry login, the subject and expiry of a TLS
//! certificate, the account of a service-account token). Never a password, a token, an `auth`
//! string or a private key.

use oxikube_domain::Resource;
use oxikube_domain::secret::{
    DecodedSecret, DecodedValue, SecretError, SecretType, docker::RegistryLogin,
};

use super::tls::{self, TlsInfo};

/// The annotation that names a service-account token's account.
const SERVICE_ACCOUNT_NAME: &str = "kubernetes.io/service-account.name";

/// One key of a Secret, without its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// The key.
    pub key: String,
    /// The decoded size in bytes.
    pub len: usize,
    /// Whether the value is not text (shown as a size even when revealed).
    pub binary: bool,
}

/// The type-specific facts of a Secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeSummary {
    /// Nothing more to say (Opaque and the types without a rendering).
    Plain,
    /// `kubernetes.io/dockerconfigjson` / `dockercfg`: the registries, usernames, no passwords.
    Registry(Vec<RegistryLogin>),
    /// A registry Secret whose config does not read as one.
    RegistryUnreadable,
    /// `kubernetes.io/tls`: the certificate.
    Tls(Box<TlsInfo>),
    /// A TLS Secret whose certificate cannot be read, with the reason.
    TlsUnreadable(String),
    /// `kubernetes.io/basic-auth`: the username (never the password).
    BasicAuth {
        /// The `username` key, when it is text.
        username: Option<String>,
    },
    /// `kubernetes.io/service-account-token`.
    ServiceAccountToken {
        /// The account the token belongs to (from the annotation).
        service_account: Option<String>,
    },
    /// `kubernetes.io/ssh-auth`: only that a private key is present.
    SshAuth,
}

/// A Secret as the detail panel shows it by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSummary {
    /// The Secret's type.
    pub secret_type: SecretType,
    /// Its keys, sorted by name.
    pub keys: Vec<KeyInfo>,
    /// What its type adds.
    pub details: TypeSummary,
    /// `immutable: true`.
    pub immutable: bool,
}

impl SecretSummary {
    /// Summarises `resource`, a full read of a Secret.
    ///
    /// # Errors
    ///
    /// [`SecretError::NotASecret`] for any other kind.
    pub fn of(resource: &Resource) -> Result<Self, SecretError> {
        let decoded = DecodedSecret::from_resource(resource)?;
        let mut keys: Vec<KeyInfo> = decoded
            .entries
            .iter()
            .map(|entry| KeyInfo {
                key: entry.key.clone(),
                len: entry.value.len(),
                binary: matches!(entry.value, DecodedValue::Binary { .. }),
            })
            .collect();
        keys.sort_by(|a, b| a.key.cmp(&b.key));
        let details = type_summary(&decoded, resource);
        Ok(Self {
            secret_type: decoded.secret_type,
            keys,
            details,
            immutable: resource
                .get("/immutable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }
}

fn type_summary(decoded: &DecodedSecret, resource: &Resource) -> TypeSummary {
    match &decoded.secret_type {
        SecretType::DockerConfigJson => registry(decoded.text(".dockerconfigjson")),
        SecretType::DockerCfg => registry(decoded.text(".dockercfg")),
        SecretType::Tls => match decoded.text("tls.crt") {
            Some(pem) => match tls::parse_certificates(pem) {
                Ok(info) => TypeSummary::Tls(Box::new(info)),
                Err(error) => TypeSummary::TlsUnreadable(error.to_string()),
            },
            None => TypeSummary::TlsUnreadable("the Secret has no `tls.crt`".into()),
        },
        SecretType::BasicAuth => TypeSummary::BasicAuth {
            username: decoded.text("username").map(str::to_owned),
        },
        SecretType::ServiceAccountToken => TypeSummary::ServiceAccountToken {
            service_account: resource
                .meta
                .annotations
                .get(SERVICE_ACCOUNT_NAME)
                .map(|name| name.to_string()),
        },
        SecretType::SshAuth => TypeSummary::SshAuth,
        _ => TypeSummary::Plain,
    }
}

fn registry(config: Option<&str>) -> TypeSummary {
    match config.and_then(oxikube_domain::secret::docker::registries) {
        Some(logins) => TypeSummary::Registry(logins),
        None => TypeSummary::RegistryUnreadable,
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::{Value, json};

    use super::super::tls::fixtures::certificate;
    use super::*;

    fn secret(kind: &str, data: Value) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "s", "namespace": "ns",
                "annotations": {"kubernetes.io/service-account.name": "builder"}},
            "type": kind, "data": data
        }))
        .unwrap()
    }

    fn b64(text: &str) -> String {
        STANDARD.encode(text)
    }

    #[test]
    fn keys_are_listed_with_sizes_and_no_values() {
        let summary =
            SecretSummary::of(&secret("Opaque", json!({"b": b64("hunter2"), "a": "//4="})))
                .unwrap();
        assert_eq!(summary.keys[0].key, "a");
        assert!(summary.keys[0].binary);
        assert_eq!(summary.keys[1].len, 7);
        assert_eq!(summary.details, TypeSummary::Plain);
        assert!(!format!("{summary:?}").contains("hunter2"));
    }

    #[test]
    fn a_registry_secret_shows_servers_and_usernames_not_passwords() {
        let config = oxikube_domain::secret::docker::config_json(
            "ghcr.io",
            "octo",
            "very-secret-token",
            None,
        );
        let summary = SecretSummary::of(&secret(
            "kubernetes.io/dockerconfigjson",
            json!({".dockerconfigjson": b64(&config)}),
        ))
        .unwrap();
        let TypeSummary::Registry(logins) = &summary.details else {
            panic!("{:?}", summary.details)
        };
        assert_eq!(logins[0].server, "ghcr.io");
        assert_eq!(logins[0].username.as_deref(), Some("octo"));
        let printed = format!("{summary:?}");
        assert!(!printed.contains("very-secret-token"), "{printed}");
        assert!(!printed.contains("b2N0bzp2ZXJ5"), "{printed}");
    }

    #[test]
    fn a_tls_secret_shows_the_certificate_and_never_the_key() {
        let (cert, key) = certificate(&["web.example.com"], 60);
        let summary = SecretSummary::of(&secret(
            "kubernetes.io/tls",
            json!({"tls.crt": b64(&cert), "tls.key": b64(&key)}),
        ))
        .unwrap();
        let TypeSummary::Tls(info) = &summary.details else {
            panic!("{:?}", summary.details)
        };
        assert!(info.subject.contains("web.example.com"));
        assert_eq!(info.dns_names, ["web.example.com"]);
        let printed = format!("{summary:?}");
        assert!(!printed.contains("PRIVATE KEY"), "{printed}");
        assert!(!printed.contains(key.lines().nth(1).unwrap()), "{printed}");
    }

    #[test]
    fn unreadable_type_data_says_so() {
        let tls = SecretSummary::of(&secret("kubernetes.io/tls", json!({}))).unwrap();
        assert!(matches!(tls.details, TypeSummary::TlsUnreadable(_)));
        let bad = SecretSummary::of(&secret(
            "kubernetes.io/tls",
            json!({"tls.crt": b64("not a cert")}),
        ))
        .unwrap();
        assert!(matches!(bad.details, TypeSummary::TlsUnreadable(_)));
        let registry = SecretSummary::of(&secret(
            "kubernetes.io/dockerconfigjson",
            json!({".dockerconfigjson": b64("{")}),
        ))
        .unwrap();
        assert_eq!(registry.details, TypeSummary::RegistryUnreadable);
    }

    #[test]
    fn basic_auth_and_service_account_types() {
        let basic = SecretSummary::of(&secret(
            "kubernetes.io/basic-auth",
            json!({"username": b64("admin"), "password": b64("pw")}),
        ))
        .unwrap();
        assert_eq!(
            basic.details,
            TypeSummary::BasicAuth {
                username: Some("admin".into())
            }
        );
        let token = SecretSummary::of(&secret(
            "kubernetes.io/service-account-token",
            json!({"token": b64("jwt")}),
        ))
        .unwrap();
        assert_eq!(
            token.details,
            TypeSummary::ServiceAccountToken {
                service_account: Some("builder".into())
            }
        );
    }
}
