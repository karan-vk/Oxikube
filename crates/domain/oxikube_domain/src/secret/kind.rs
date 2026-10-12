//! [`SecretType`]: the `type` of a Secret, which says what its keys mean.

/// The type of a Secret (`spec.type`), with the types Kubernetes defines named.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum SecretType {
    /// `Opaque`: arbitrary user data (the default when `type` is absent).
    #[default]
    Opaque,
    /// `kubernetes.io/dockerconfigjson`: a registry login in `.dockerconfigjson`.
    DockerConfigJson,
    /// `kubernetes.io/dockercfg`: the legacy registry login in `.dockercfg`.
    DockerCfg,
    /// `kubernetes.io/tls`: `tls.crt` and `tls.key`.
    Tls,
    /// `kubernetes.io/service-account-token`: a ServiceAccount token.
    ServiceAccountToken,
    /// `kubernetes.io/basic-auth`: `username` and `password`.
    BasicAuth,
    /// `kubernetes.io/ssh-auth`: `ssh-privatekey`.
    SshAuth,
    /// `bootstrap.kubernetes.io/token`: a bootstrap token.
    BootstrapToken,
    /// Any other type (a Helm release, a custom controller's).
    Other(String),
}

impl SecretType {
    /// The type named by `spec.type`; absent or empty is `Opaque`.
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or_default() {
            "" | "Opaque" => SecretType::Opaque,
            "kubernetes.io/dockerconfigjson" => SecretType::DockerConfigJson,
            "kubernetes.io/dockercfg" => SecretType::DockerCfg,
            "kubernetes.io/tls" => SecretType::Tls,
            "kubernetes.io/service-account-token" => SecretType::ServiceAccountToken,
            "kubernetes.io/basic-auth" => SecretType::BasicAuth,
            "kubernetes.io/ssh-auth" => SecretType::SshAuth,
            "bootstrap.kubernetes.io/token" => SecretType::BootstrapToken,
            other => SecretType::Other(other.to_owned()),
        }
    }

    /// The `type` string as the API writes it.
    pub fn as_str(&self) -> &str {
        match self {
            SecretType::Opaque => "Opaque",
            SecretType::DockerConfigJson => "kubernetes.io/dockerconfigjson",
            SecretType::DockerCfg => "kubernetes.io/dockercfg",
            SecretType::Tls => "kubernetes.io/tls",
            SecretType::ServiceAccountToken => "kubernetes.io/service-account-token",
            SecretType::BasicAuth => "kubernetes.io/basic-auth",
            SecretType::SshAuth => "kubernetes.io/ssh-auth",
            SecretType::BootstrapToken => "bootstrap.kubernetes.io/token",
            SecretType::Other(other) => other,
        }
    }

    /// A short name for a badge: `Opaque`, `Docker registry`, `TLS`, ...
    pub fn label(&self) -> &str {
        match self {
            SecretType::Opaque => "Opaque",
            SecretType::DockerConfigJson | SecretType::DockerCfg => "Docker registry",
            SecretType::Tls => "TLS",
            SecretType::ServiceAccountToken => "Service account token",
            SecretType::BasicAuth => "Basic auth",
            SecretType::SshAuth => "SSH auth",
            SecretType::BootstrapToken => "Bootstrap token",
            SecretType::Other(other) => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_round_trip_through_their_strings() {
        for text in [
            "Opaque",
            "kubernetes.io/dockerconfigjson",
            "kubernetes.io/dockercfg",
            "kubernetes.io/tls",
            "kubernetes.io/service-account-token",
            "kubernetes.io/basic-auth",
            "kubernetes.io/ssh-auth",
            "bootstrap.kubernetes.io/token",
            "helm.sh/release.v1",
        ] {
            assert_eq!(SecretType::parse(Some(text)).as_str(), text);
        }
        assert_eq!(SecretType::parse(None), SecretType::Opaque);
        assert_eq!(SecretType::parse(Some("")), SecretType::Opaque);
    }
}
