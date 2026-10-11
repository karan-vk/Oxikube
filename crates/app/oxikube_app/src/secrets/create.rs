//! A new Secret, validated before anything is sent (E12-S11): Opaque key/value rows, a Docker
//! registry login or a TLS pair.
//!
//! [`NewSecret`] is what the "Create secret" dialog collects. [`NewSecret::validate`] lists
//! every problem with the field it belongs to, so the dialog can mark them all at once, and
//! [`NewSecret::manifest`] builds the Secret object (`data` base64, as the API stores it) once
//! it is valid. The object is made through the editor and `resource::Apply`, never here.

use std::collections::HashSet;

use oxikube_domain::secret::{SecretText, SecretType, docker, is_displayable};
use serde_json::{Map, Value, json};

use super::tls;

/// The largest a Secret's data may be: the API server's 1 MiB limit on the object.
pub const MAX_SECRET_BYTES: usize = 1024 * 1024;

/// A Secret to create. `Debug` shows names and `***`, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewSecret {
    /// `type: Opaque`: arbitrary key/value pairs.
    Opaque {
        /// The Secret's name.
        name: String,
        /// Its namespace.
        namespace: String,
        /// The keys and their (plaintext) values.
        entries: Vec<(String, SecretText)>,
    },
    /// `type: kubernetes.io/dockerconfigjson`: one registry login.
    DockerRegistry {
        /// The Secret's name.
        name: String,
        /// Its namespace.
        namespace: String,
        /// The registry (`ghcr.io`).
        server: String,
        /// The username.
        username: String,
        /// The password or token.
        password: SecretText,
        /// An email, optional.
        email: String,
    },
    /// `type: kubernetes.io/tls`: a certificate and its private key.
    Tls {
        /// The Secret's name.
        name: String,
        /// Its namespace.
        namespace: String,
        /// The certificate (chain) as PEM.
        certificate: SecretText,
        /// The private key as PEM.
        private_key: SecretText,
    },
}

/// The field of the dialog a [`Problem`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    /// The Secret's name.
    Name,
    /// Its namespace.
    Namespace,
    /// The key of the nth row (Opaque).
    Key(usize),
    /// The value of the nth row (Opaque).
    Value(usize),
    /// The registry.
    Server,
    /// The registry username.
    Username,
    /// The registry password.
    Password,
    /// The registry email.
    Email,
    /// The certificate PEM.
    Certificate,
    /// The private key PEM.
    PrivateKey,
    /// The Secret as a whole (too large, no rows).
    Secret,
}

/// One thing wrong with a [`NewSecret`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Where it is.
    pub field: Field,
    /// What is wrong, as a sentence.
    pub message: String,
}

impl Problem {
    fn new(field: Field, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl NewSecret {
    /// The Secret's type.
    pub fn secret_type(&self) -> SecretType {
        match self {
            NewSecret::Opaque { .. } => SecretType::Opaque,
            NewSecret::DockerRegistry { .. } => SecretType::DockerConfigJson,
            NewSecret::Tls { .. } => SecretType::Tls,
        }
    }

    fn name(&self) -> &str {
        match self {
            NewSecret::Opaque { name, .. }
            | NewSecret::DockerRegistry { name, .. }
            | NewSecret::Tls { name, .. } => name,
        }
    }

    fn namespace(&self) -> &str {
        match self {
            NewSecret::Opaque { namespace, .. }
            | NewSecret::DockerRegistry { namespace, .. }
            | NewSecret::Tls { namespace, .. } => namespace,
        }
    }

    /// Every problem with the Secret, empty when it can be created.
    pub fn validate(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        if let Err(message) = dns_subdomain(self.name()) {
            problems.push(Problem::new(Field::Name, message));
        }
        if let Err(message) = dns_label(self.namespace()) {
            problems.push(Problem::new(Field::Namespace, message));
        }
        match self {
            NewSecret::Opaque { entries, .. } => validate_entries(entries, &mut problems),
            NewSecret::DockerRegistry {
                server,
                username,
                password,
                email,
                ..
            } => validate_registry(server, username, password, email, &mut problems),
            NewSecret::Tls {
                certificate,
                private_key,
                ..
            } => validate_tls(certificate, private_key, &mut problems),
        }
        if problems.is_empty() && self.data_bytes() > MAX_SECRET_BYTES {
            problems.push(Problem::new(
                Field::Secret,
                "a Secret may hold at most 1 MiB",
            ));
        }
        problems
    }

    fn data_bytes(&self) -> usize {
        match self {
            NewSecret::Opaque { entries, .. } => {
                entries.iter().map(|(k, v)| k.len() + v.len()).sum()
            }
            NewSecret::DockerRegistry { password, .. } => password.len() + 256,
            NewSecret::Tls {
                certificate,
                private_key,
                ..
            } => certificate.len() + private_key.len(),
        }
    }

    /// The Secret object, `data` base64 as the API stores it.
    ///
    /// # Errors
    ///
    /// The [`Problem`]s of [`validate`](Self::validate) when there are any.
    pub fn manifest(&self) -> Result<Value, Vec<Problem>> {
        let problems = self.validate();
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut data: Vec<(String, SecretText)> = Vec::new();
        match self {
            NewSecret::Opaque { entries, .. } => {
                data.extend(entries.iter().cloned());
            }
            NewSecret::DockerRegistry {
                server,
                username,
                password,
                email,
                ..
            } => {
                let email = (!email.is_empty()).then_some(email.as_str());
                let config = docker::config_json(server, username, password.expose(), email);
                data.push((".dockerconfigjson".into(), SecretText::new(config)));
            }
            NewSecret::Tls {
                certificate,
                private_key,
                ..
            } => {
                data.push(("tls.crt".into(), certificate.clone()));
                data.push(("tls.key".into(), private_key.clone()));
            }
        }
        let encoded: Map<String, Value> = data
            .into_iter()
            .map(|(key, value)| {
                let value = oxikube_domain::secret::DecodedValue::Text(value).to_base64();
                (key, Value::String(value))
            })
            .collect();
        Ok(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {"name": self.name(), "namespace": self.namespace()},
            "type": self.secret_type().as_str(),
            "data": encoded,
        }))
    }
}

/// The message for text the editor would show as `(binary, N bytes)`.
const CONTROL_CHARACTERS: &str = "the text has control characters other than tabs and line breaks";

fn validate_entries(entries: &[(String, SecretText)], problems: &mut Vec<Problem>) {
    if entries.is_empty() {
        problems.push(Problem::new(Field::Secret, "add at least one key"));
    }
    let mut seen = HashSet::new();
    for (index, (key, value)) in entries.iter().enumerate() {
        if let Err(message) = secret_key(key) {
            problems.push(Problem::new(Field::Key(index), message));
        } else if !seen.insert(key.as_str()) {
            problems.push(Problem::new(Field::Key(index), "this key is used twice"));
        }
        if value.is_empty() {
            problems.push(Problem::new(Field::Value(index), "the value is empty"));
        } else if !is_displayable(value.expose()) {
            // The editor shows such a value as `(binary, N bytes)` and a new Secret has no original
            // to keep it from: it would be stored as that text.
            problems.push(Problem::new(Field::Value(index), CONTROL_CHARACTERS));
        }
    }
}

fn validate_registry(
    server: &str,
    username: &str,
    password: &SecretText,
    email: &str,
    problems: &mut Vec<Problem>,
) {
    if server.trim().is_empty() {
        problems.push(Problem::new(Field::Server, "the registry is required"));
    } else if server.contains(char::is_whitespace) {
        problems.push(Problem::new(
            Field::Server,
            "the registry has a space in it",
        ));
    }
    if username.is_empty() {
        problems.push(Problem::new(Field::Username, "the username is required"));
    }
    if password.is_empty() {
        problems.push(Problem::new(Field::Password, "the password is required"));
    }
    if !email.is_empty() && !(email.contains('@') && !email.contains(char::is_whitespace)) {
        problems.push(Problem::new(Field::Email, "this is not an email address"));
    }
}

fn validate_tls(certificate: &SecretText, private_key: &SecretText, problems: &mut Vec<Problem>) {
    if certificate.is_empty() {
        problems.push(Problem::new(
            Field::Certificate,
            "the certificate is required",
        ));
    } else if !is_displayable(certificate.expose()) {
        problems.push(Problem::new(Field::Certificate, CONTROL_CHARACTERS));
    } else if let Err(error) = tls::parse_certificates(certificate.expose()) {
        problems.push(Problem::new(Field::Certificate, error.to_string()));
    }
    if private_key.is_empty() {
        problems.push(Problem::new(
            Field::PrivateKey,
            "the private key is required",
        ));
    } else if !is_displayable(private_key.expose()) {
        // The PEM reader skips text outside a block, so it cannot catch this.
        problems.push(Problem::new(Field::PrivateKey, CONTROL_CHARACTERS));
    } else if let Err(error) = tls::check_private_key(private_key.expose()) {
        problems.push(Problem::new(Field::PrivateKey, error.to_string()));
    }
}

/// A Secret key: letters, digits, `-`, `_` and `.`, at most 253 characters, not `.` or `..`.
fn secret_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("the key is required".into());
    }
    if key == "." || key == ".." {
        return Err("a key cannot be `.` or `..`".into());
    }
    if key.len() > 253 {
        return Err("a key is at most 253 characters".into());
    }
    if !key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("a key may hold letters, digits, `-`, `_` and `.`".into());
    }
    Ok(())
}

/// A DNS-1123 subdomain: the rule for object names.
fn dns_subdomain(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the name is required".into());
    }
    if name.len() > 253 {
        return Err("a name is at most 253 characters".into());
    }
    let ok_char = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.';
    let edge = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let bytes = name.as_bytes();
    if !bytes.iter().copied().all(ok_char)
        || !bytes.first().is_some_and(edge)
        || !bytes.last().is_some_and(edge)
    {
        return Err(
            "use lowercase letters, digits, `-` and `.`, starting and ending with a letter or digit"
                .into(),
        );
    }
    Ok(())
}

/// A DNS-1123 label: the rule for namespace names.
fn dns_label(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the namespace is required".into());
    }
    if name.len() > 63 {
        return Err("a namespace is at most 63 characters".into());
    }
    let bytes = name.as_bytes();
    let edge = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !bytes.iter().all(|b| edge(b) || *b == b'-')
        || !bytes.first().is_some_and(edge)
        || !bytes.last().is_some_and(edge)
    {
        return Err(
            "use lowercase letters, digits and `-`, starting and ending with a letter or digit"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use oxikube_domain::secret::DecodedSecret;

    use super::super::tls::fixtures::certificate;
    use super::*;

    fn opaque(entries: &[(&str, &str)]) -> NewSecret {
        NewSecret::Opaque {
            name: "app-secret".into(),
            namespace: "default".into(),
            entries: entries
                .iter()
                .map(|(k, v)| ((*k).to_owned(), SecretText::new(*v)))
                .collect(),
        }
    }

    fn fields(secret: &NewSecret) -> Vec<Field> {
        secret.validate().into_iter().map(|p| p.field).collect()
    }

    #[test]
    fn an_opaque_secret_becomes_a_manifest_with_base64_data() {
        let manifest = opaque(&[("password", "hunter2"), ("user", "admin")])
            .manifest()
            .unwrap();
        assert_eq!(manifest["kind"], "Secret");
        assert_eq!(manifest["type"], "Opaque");
        assert_eq!(manifest["metadata"]["name"], "app-secret");
        assert_eq!(manifest["metadata"]["namespace"], "default");
        assert_eq!(manifest["data"]["password"], "aHVudGVyMg==");
        let decoded = DecodedSecret::from_value(&manifest).unwrap();
        assert_eq!(decoded.text("user"), Some("admin"));
    }

    #[test]
    fn opaque_rows_are_checked_one_by_one() {
        assert_eq!(fields(&opaque(&[])), [Field::Secret]);
        assert_eq!(
            fields(&opaque(&[
                ("a b", "x"),
                ("a b", "y"),
                ("ok", ""),
                ("..", "z")
            ])),
            [Field::Key(0), Field::Key(1), Field::Value(2), Field::Key(3)]
        );
        let twice = opaque(&[("same", "1"), ("same", "2")]);
        assert_eq!(fields(&twice), [Field::Key(1)]);
    }

    #[test]
    fn a_value_with_control_characters_is_refused_not_turned_into_a_placeholder() {
        let secret = opaque(&[
            ("ok", "fine\nmultiline\t"),
            ("esc", "tok\u{1b}en"),
            ("nul", "a\0b"),
        ]);
        assert_eq!(fields(&secret), [Field::Value(1), Field::Value(2)]);
        assert!(secret.manifest().is_err());
    }

    #[test]
    fn names_follow_the_dns_rules() {
        let mut secret = opaque(&[("a", "b")]);
        for bad in [
            "",
            "Upper",
            "-lead",
            "trail-",
            "under_score",
            &"x".repeat(254),
        ] {
            if let NewSecret::Opaque { name, .. } = &mut secret {
                *name = bad.to_owned();
            }
            assert_eq!(fields(&secret), [Field::Name], "{bad:?}");
        }
        if let NewSecret::Opaque {
            name, namespace, ..
        } = &mut secret
        {
            *name = "a.b-c".into();
            *namespace = "Has.Dot".into();
        }
        assert_eq!(fields(&secret), [Field::Namespace]);
    }

    #[test]
    fn a_registry_login_becomes_a_dockerconfigjson_with_auth() {
        let secret = NewSecret::DockerRegistry {
            name: "pull".into(),
            namespace: "default".into(),
            server: "ghcr.io".into(),
            username: "octo".into(),
            password: SecretText::new("p4ss"),
            email: "o@example.com".into(),
        };
        let manifest = secret.manifest().unwrap();
        assert_eq!(manifest["type"], "kubernetes.io/dockerconfigjson");
        let decoded = DecodedSecret::from_value(&manifest).unwrap();
        let config = decoded.text(".dockerconfigjson").unwrap();
        let parsed: Value = serde_json::from_str(config).unwrap();
        assert_eq!(parsed["auths"]["ghcr.io"]["username"], "octo");
        // base64("octo:p4ss")
        assert_eq!(parsed["auths"]["ghcr.io"]["auth"], "b2N0bzpwNHNz");
        let logins = docker::registries(config).unwrap();
        assert_eq!(logins[0].username.as_deref(), Some("octo"));
    }

    #[test]
    fn registry_fields_are_required() {
        let secret = NewSecret::DockerRegistry {
            name: "pull".into(),
            namespace: "default".into(),
            server: " ".into(),
            username: String::new(),
            password: SecretText::new(""),
            email: "nope".into(),
        };
        assert_eq!(
            fields(&secret),
            [
                Field::Server,
                Field::Username,
                Field::Password,
                Field::Email
            ]
        );
    }

    #[test]
    fn a_tls_pair_is_validated_as_pem_before_it_is_sent() {
        let (cert, key) = certificate(&["web.example.com"], 30);
        let good = NewSecret::Tls {
            name: "web-tls".into(),
            namespace: "default".into(),
            certificate: SecretText::new(cert.clone()),
            private_key: SecretText::new(key.clone()),
        };
        let manifest = good.manifest().unwrap();
        assert_eq!(manifest["type"], "kubernetes.io/tls");
        let decoded = DecodedSecret::from_value(&manifest).unwrap();
        assert_eq!(decoded.text("tls.crt"), Some(cert.as_str()));
        assert_eq!(decoded.text("tls.key"), Some(key.as_str()));

        let bad = NewSecret::Tls {
            name: "web-tls".into(),
            namespace: "default".into(),
            certificate: SecretText::new("hello"),
            private_key: SecretText::new(cert),
        };
        assert_eq!(fields(&bad), [Field::Certificate, Field::PrivateKey]);
        assert!(bad.manifest().is_err());
        let swapped = NewSecret::Tls {
            name: "web-tls".into(),
            namespace: "default".into(),
            certificate: SecretText::new(key),
            private_key: SecretText::new(""),
        };
        assert_eq!(fields(&swapped), [Field::Certificate, Field::PrivateKey]);
    }

    #[test]
    fn a_tls_pair_with_control_characters_outside_the_pem_blocks_is_refused() {
        let (cert, key) = certificate(&["web.example.com"], 30);
        let tls = |certificate: String, private_key: String| NewSecret::Tls {
            name: "web-tls".into(),
            namespace: "default".into(),
            certificate: SecretText::new(certificate),
            private_key: SecretText::new(private_key),
        };
        // The PEM reader skips lines outside a block, so only the displayable check catches these.
        let dirty = tls(format!("\u{1b}[0m\n{cert}"), format!("{key}\n\0"));
        assert_eq!(fields(&dirty), [Field::Certificate, Field::PrivateKey]);
        assert!(dirty.manifest().is_err());
        assert!(
            tls(format!("{cert}\n"), format!("{key}\t\n"))
                .manifest()
                .is_ok()
        );
    }

    #[test]
    fn debug_never_prints_a_value() {
        let secret = opaque(&[("password", "hunter2")]);
        let printed = format!("{secret:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(printed.contains("password") && printed.contains("***"));
    }

    #[test]
    fn a_secret_over_one_mebibyte_is_refused() {
        let big = "x".repeat(MAX_SECRET_BYTES);
        assert_eq!(fields(&opaque(&[("big", &big)])), [Field::Secret]);
    }
}
