//! Classification of client-configuration errors: kubeconfig resolution and the
//! rustls setup kube does while building a client.
//!
//! These errors happen before any request, so none of them is `Network`. Their kube
//! messages are never echoed wholesale: a base64 decode error names an offending byte
//! and offset of `client-key-data`, and a proxy URL can carry `user:password@`. A
//! source is attached only for variants whose text is known to hold no credential
//! material (names, paths, URL parse errors), and `OxiError`'s `Debug` prints it.

use kube::client::RustlsTlsError;
use kube::config::KubeconfigError;
use oxikube_domain::OxiError;

/// Classifies a kubeconfig resolution error (`Config::from_custom_kubeconfig`, or
/// `kube::Error::InferKubeconfig` when the client identity is loaded at build time).
pub fn classify_kubeconfig(err: &KubeconfigError) -> OxiError {
    use KubeconfigError as K;
    let invalid_entry = || OxiError::validation("invalid kubeconfig entry");
    let (base, safe_source) = match err {
        K::LoadContext(_) => (
            OxiError::not_found("the context has no definition in the kubeconfig"),
            true,
        ),
        K::ParseProxyUrl(_) => (
            OxiError::validation(
                "the proxy URL (kubeconfig `proxy-url`, or `HTTPS_PROXY` / `https_proxy` in \
                 the environment) is not a valid URI",
            ),
            false,
        ),
        K::LoadClientKey(_) => (
            OxiError::validation("the client key in the kubeconfig could not be loaded"),
            false,
        ),
        K::LoadClientCertificate(_) => (
            OxiError::validation("the client certificate in the kubeconfig could not be loaded"),
            false,
        ),
        K::LoadCertificateAuthority(_) | K::ParseCertificates(_) => (
            OxiError::validation("the certificate authority in the kubeconfig could not be loaded"),
            false,
        ),
        K::CurrentContextNotSet
        | K::KindMismatch
        | K::ApiVersionMismatch
        | K::LoadClusterOfContext(_)
        | K::FindPath
        | K::ReadConfig(..)
        | K::MissingClusterUrl
        | K::ParseClusterUrl(_) => (invalid_entry(), true),
        // `Parse` can quote YAML, credentials included; future variants get the same
        // conservative treatment.
        #[allow(unreachable_patterns)]
        _ => (invalid_entry(), false),
    };
    if safe_source {
        base.with_source(err.to_string())
    } else {
        base
    }
}

/// Classifies a rustls setup error (`kube::Error::RustlsTls`). kube raises these while
/// building the TLS connector, not during a request; handshake failures on a request
/// arrive as transport errors instead.
pub fn classify_tls_setup(err: &RustlsTlsError) -> OxiError {
    use RustlsTlsError as T;
    match err {
        T::InvalidIdentityPem(_) | T::InvalidPrivateKey(_) | T::UnknownPrivateKeyFormat => {
            OxiError::validation("the client certificate or key in the kubeconfig is invalid")
        }
        T::MissingPrivateKey => OxiError::validation(
            "the client identity in the kubeconfig has no private key (PKCS8 or RSA/PKCS1 required)",
        ),
        T::MissingCertificate => {
            OxiError::validation("the client identity in the kubeconfig has no certificate")
        }
        T::AddRootCertificate(_) => {
            OxiError::validation("the certificate authority in the kubeconfig could not be used")
        }
        T::NoValidNativeRootCA(_) => OxiError::internal(
            "no usable system root certificates were found; set `certificate-authority` in the \
             kubeconfig",
        ),
        T::InvalidServerName(_) => OxiError::validation(
            "the TLS server name (`tls-server-name` or the server host) is not valid",
        ),
        #[allow(unreachable_patterns)]
        _ => OxiError::validation("the TLS settings in the kubeconfig are invalid"),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use base64::Engine as _;
    use kube::{Client, Config};
    use oxikube_domain::ErrorKind;

    use super::*;
    use crate::auth::classify;

    /// `!` (byte 33) is not base64; kube's decode error would name it and its offset.
    const BAD_B64: &str = "c2VjcmV0!S0VZ";

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    fn client_error(cert: &str, key: &str) -> kube::Error {
        let mut config = Config::new("https://127.0.0.1:1".parse().expect("uri"));
        config.accept_invalid_certs = true;
        config.auth_info.client_certificate_data = Some(cert.to_owned());
        config.auth_info.client_key_data = Some(key.to_owned().into());
        match Client::try_from(config) {
            Ok(_) => panic!("the malformed identity was accepted"),
            Err(err) => err,
        }
    }

    fn assert_no_leak(err: &OxiError) {
        let text = format!("{err} {err:?}");
        for leak in [
            "c2VjcmV0",
            "S0VZ",
            "33",
            "InvalidCharacter",
            "Invalid symbol",
            "offset",
        ] {
            assert!(!text.contains(leak), "leaked {leak:?} in {text}");
        }
        assert!(err.source().is_none(), "{err:?}");
    }

    #[test]
    fn bad_base64_in_client_key_data_is_validation_without_key_bytes() {
        let err = client_error(&b64("-----BEGIN CERTIFICATE-----\n"), BAD_B64);
        assert!(matches!(err, kube::Error::InferKubeconfig(_)), "{err:?}");
        let classified = classify(&err);
        assert_eq!(classified.kind(), ErrorKind::Validation);
        assert!(classified.message().contains("client key"), "{classified}");
        assert_no_leak(&classified);
    }

    #[test]
    fn bad_pem_body_in_client_key_is_validation_not_network() {
        let cert = b64("-----BEGIN CERTIFICATE-----\nTUlJQg==\n-----END CERTIFICATE-----\n");
        let key = b64(&format!(
            "-----BEGIN PRIVATE KEY-----\n{BAD_B64}\n-----END PRIVATE KEY-----\n"
        ));
        let err = client_error(&cert, &key);
        assert!(matches!(err, kube::Error::RustlsTls(_)), "{err:?}");
        let classified = classify(&err);
        assert_eq!(classified.kind(), ErrorKind::Validation);
        assert!(!classified.is_retryable());
        assert_no_leak(&classified);
    }

    #[test]
    fn every_tls_setup_error_is_a_fixed_message() {
        for err in [
            RustlsTlsError::MissingPrivateKey,
            RustlsTlsError::MissingCertificate,
            RustlsTlsError::UnknownPrivateKeyFormat,
        ] {
            let classified = classify_tls_setup(&err);
            assert_eq!(classified.kind(), ErrorKind::Validation, "{err}");
            assert!(classified.source().is_none());
        }
        let native = RustlsTlsError::NoValidNativeRootCA(std::io::Error::other("no roots"));
        assert_eq!(classify_tls_setup(&native).kind(), ErrorKind::Internal);
    }

    fn kubeconfig_error(cluster_extra: &str) -> KubeconfigError {
        let yaml = format!(
            "apiVersion: v1\nkind: Config\nclusters:\n- name: k\n  cluster:\n{cluster_extra}\n\
             contexts:\n- name: k\n  context: {{cluster: k}}\n"
        );
        let kubeconfig = kube::config::Kubeconfig::from_yaml(&yaml).expect("yaml");
        let options = kube::config::KubeConfigOptions {
            context: Some("k".into()),
            ..Default::default()
        };
        match futures::executor::block_on(Config::from_custom_kubeconfig(kubeconfig, &options)) {
            Ok(_) => panic!("the broken cluster was accepted"),
            Err(err) => err,
        }
    }

    #[test]
    fn certificate_authority_errors_carry_no_source() {
        let err = kubeconfig_error(&format!(
            "    server: https://127.0.0.1:1\n    certificate-authority-data: \"{BAD_B64}\""
        ));
        let classified = classify_kubeconfig(&err);
        assert_eq!(classified.kind(), ErrorKind::Validation);
        assert!(classified.message().contains("certificate authority"));
        assert_no_leak(&classified);
    }

    #[test]
    fn proxy_errors_never_echo_the_url() {
        let err = kubeconfig_error(
            "    server: https://127.0.0.1:1\n    proxy-url: \"http://user:pa ss@bad host\"",
        );
        let classified = classify_kubeconfig(&err);
        assert_eq!(classified.kind(), ErrorKind::Validation);
        let text = format!("{classified:?}");
        assert!(
            !text.contains("pa ss") && !text.contains("bad host"),
            "{text}"
        );
        assert!(classified.source().is_none());
    }

    #[test]
    fn safe_kubeconfig_errors_keep_their_text_as_source() {
        let err = kubeconfig_error("    insecure-skip-tls-verify: true");
        assert!(matches!(err, KubeconfigError::MissingClusterUrl), "{err:?}");
        let classified = classify_kubeconfig(&err);
        assert_eq!(classified.kind(), ErrorKind::Validation);
        assert!(classified.source().is_some());
    }
}
