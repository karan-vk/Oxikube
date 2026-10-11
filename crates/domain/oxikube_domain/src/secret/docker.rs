//! Registry logins: reading `.dockerconfigjson` / `.dockercfg` for display, and writing one.
//!
//! A registry Secret holds passwords (and `auth`, which is `base64(user:password)`). The summary
//! [`registries`] returns therefore carries the server, the username and whether credentials are
//! present, never the password or the `auth` string; those are shown only when the user reveals
//! the Secret.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};

/// One registry of a Docker config, without its credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryLogin {
    /// The registry (`ghcr.io`, `https://index.docker.io/v1/`).
    pub server: String,
    /// The username, from `username` or the left side of `auth`.
    pub username: Option<String>,
    /// The email, when one is set.
    pub email: Option<String>,
    /// Whether the entry carries a password or an `auth` string.
    pub has_credentials: bool,
}

/// The registries of a Docker config: `.dockerconfigjson` (`{"auths": {server: {...}}}`) or the
/// legacy `.dockercfg` (`{server: {...}}`). `None` when the text is not JSON of that shape.
pub fn registries(config: &str) -> Option<Vec<RegistryLogin>> {
    let value: Value = serde_json::from_str(config).ok()?;
    let object = value.as_object()?;
    let auths = match object.get("auths") {
        Some(auths) => auths.as_object()?,
        None => object,
    };
    Some(
        auths
            .iter()
            .filter_map(|(server, entry)| {
                let entry = entry.as_object()?;
                let text = |field: &str| {
                    entry
                        .get(field)
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                };
                let from_auth = text("auth").and_then(|auth| {
                    let decoded = STANDARD.decode(auth.trim()).ok()?;
                    let decoded = String::from_utf8(decoded).ok()?;
                    decoded.split_once(':').map(|(user, _)| user.to_owned())
                });
                Some(RegistryLogin {
                    server: server.clone(),
                    username: text("username").map(str::to_owned).or(from_auth),
                    email: text("email").map(str::to_owned),
                    has_credentials: text("password").is_some() || text("auth").is_some(),
                })
            })
            .collect(),
    )
}

/// The `.dockerconfigjson` text for one registry login, as `kubectl create secret
/// docker-registry` writes it: `username`, `password`, `email` (when given) and `auth`
/// (`base64(username:password)`).
pub fn config_json(server: &str, username: &str, password: &str, email: Option<&str>) -> String {
    let mut entry = Map::new();
    entry.insert("username".into(), json!(username));
    entry.insert("password".into(), json!(password));
    if let Some(email) = email.filter(|email| !email.is_empty()) {
        entry.insert("email".into(), json!(email));
    }
    entry.insert(
        "auth".into(),
        json!(STANDARD.encode(format!("{username}:{password}"))),
    );
    json!({"auths": {server: entry}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_config_reads_back_without_the_password() {
        let text = config_json("ghcr.io", "octo", "p4ss", Some("o@example.com"));
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed["auths"]["ghcr.io"]["auth"],
            STANDARD.encode("octo:p4ss")
        );
        let logins = registries(&text).unwrap();
        assert_eq!(
            logins,
            vec![RegistryLogin {
                server: "ghcr.io".into(),
                username: Some("octo".into()),
                email: Some("o@example.com".into()),
                has_credentials: true,
            }]
        );
        assert!(!format!("{logins:?}").contains("p4ss"));
    }

    #[test]
    fn the_username_comes_from_auth_when_there_is_no_field() {
        let auth = STANDARD.encode("bob:secret");
        let text = format!(r#"{{"auths": {{"r.example": {{"auth": "{auth}"}}}}}}"#);
        let logins = registries(&text).unwrap();
        assert_eq!(logins[0].username.as_deref(), Some("bob"));
        assert!(logins[0].has_credentials);
    }

    #[test]
    fn the_legacy_shape_and_bad_input() {
        let legacy = r#"{"r.example": {"username": "u", "password": "p"}}"#;
        assert_eq!(registries(legacy).unwrap()[0].server, "r.example");
        assert!(registries("not json").is_none());
        assert!(registries("[1, 2]").is_none());
        assert_eq!(registries(r#"{"auths": {}}"#).unwrap(), vec![]);
    }
}
