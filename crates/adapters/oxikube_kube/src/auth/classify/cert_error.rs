//! Certificate rejections in the TLS handshake.
//!
//! kube reports a failed handshake as a transport error (`Error::Service` or
//! `Error::HyperError`). Somewhere down its chain sits the `rustls::Error`, usually inside
//! the `io::Error` that tokio-rustls wraps it in. `io::Error::source()` skips the error it
//! wraps (it returns that error's own source), so the walk below also looks through
//! [`io::Error::get_ref`].
//!
//! A rejected certificate does not fix itself by retrying: the result is a `Network`
//! error with `retryable = false`, which the health policy treats as permanent. The
//! message names only the reason (the `CertificateError` variant), never the certificate,
//! the names it presented or the server URL.

use std::error::Error as StdError;
use std::io;

use oxikube_domain::OxiError;
use rustls::CertificateError;

/// More hops than any real transport stack; guards against a pathological chain.
const MAX_HOPS: usize = 32;

/// The classification of a certificate rejection anywhere in `err`'s chain, if there is one.
pub(super) fn certificate_rejection(err: &(dyn StdError + 'static)) -> Option<OxiError> {
    let mut cur: Option<&(dyn StdError + 'static)> = Some(err);
    for _ in 0..MAX_HOPS {
        let e = cur?;
        if let Some(tls) = e.downcast_ref::<rustls::Error>() {
            return rejection(tls);
        }
        cur = match e.downcast_ref::<io::Error>().and_then(io::Error::get_ref) {
            Some(inner) => Some(inner as &(dyn StdError + 'static)),
            None => e.source(),
        };
    }
    None
}

fn rejection(err: &rustls::Error) -> Option<OxiError> {
    let (reason, hint) = match err {
        rustls::Error::InvalidCertificate(cert) => certificate_reason(cert),
        rustls::Error::NoCertificatesPresented => (
            "NoCertificatesPresented",
            "the server presented no certificate",
        ),
        _ => return None,
    };
    Some(
        OxiError::network(format!(
            "the cluster's TLS certificate was rejected ({reason}): {hint}"
        ))
        .with_retryable(false),
    )
}

/// The variant name and a fixed hint. The `*Context` variants carry times, names and
/// algorithm ids; they are folded into their plain variant and none of the data is used.
#[allow(deprecated)] // `UnsupportedSignatureAlgorithm` is deprecated but still constructible.
fn certificate_reason(err: &CertificateError) -> (&'static str, &'static str) {
    use CertificateError as C;
    match err {
        C::UnknownIssuer => (
            "UnknownIssuer",
            "it is not issued by a trusted certificate authority; check the cluster's \
             certificate-authority in the kubeconfig",
        ),
        C::Expired | C::ExpiredContext { .. } => ("Expired", "the server certificate has expired"),
        C::NotValidYet | C::NotValidYetContext { .. } => (
            "NotValidYet",
            "the server certificate is not valid yet; check the system clock",
        ),
        C::NotValidForName | C::NotValidForNameContext { .. } => (
            "NotValidForName",
            "the server certificate does not match the cluster address",
        ),
        C::Revoked => ("Revoked", "the server certificate has been revoked"),
        C::BadEncoding => ("BadEncoding", "the server certificate is malformed"),
        C::BadSignature => ("BadSignature", "the certificate chain has a bad signature"),
        C::UnsupportedSignatureAlgorithm
        | C::UnsupportedSignatureAlgorithmContext { .. }
        | C::UnsupportedSignatureAlgorithmForPublicKeyContext { .. } => (
            "UnsupportedSignatureAlgorithm",
            "the certificate is signed with an unsupported algorithm",
        ),
        C::InvalidPurpose | C::InvalidPurposeContext { .. } => (
            "InvalidPurpose",
            "the server certificate is not valid for TLS server authentication",
        ),
        C::UnhandledCriticalExtension => (
            "UnhandledCriticalExtension",
            "the server certificate has an unsupported critical extension",
        ),
        C::UnknownRevocationStatus
        | C::ExpiredRevocationList
        | C::ExpiredRevocationListContext { .. } => (
            "UnknownRevocationStatus",
            "the server certificate's revocation status could not be checked",
        ),
        C::InvalidOcspResponse => ("InvalidOcspResponse", "the OCSP response was invalid"),
        C::ApplicationVerificationFailure => (
            "ApplicationVerificationFailure",
            "the certificate verifier rejected the server certificate",
        ),
        // `Other` (also what the macOS platform verifier returns for most trust failures)
        // wraps free text that can name the certificate; it is not shown.
        _ => (
            "Other",
            "the certificate verifier rejected the server certificate",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped(tls: rustls::Error) -> Box<dyn StdError + Send + Sync> {
        // The shape tokio-rustls produces, under one more layer as hyper-util adds.
        Box::new(io::Error::other(io::Error::new(
            io::ErrorKind::InvalidData,
            tls,
        )))
    }

    #[test]
    fn finds_a_certificate_error_inside_io_errors() {
        let err = wrapped(rustls::Error::InvalidCertificate(
            CertificateError::UnknownIssuer,
        ));
        let oxi = certificate_rejection(err.as_ref()).expect("a rejection");
        assert!(!oxi.is_retryable());
        assert!(oxi.message().contains("UnknownIssuer"), "{oxi}");
    }

    #[test]
    fn context_variants_do_not_leak_their_data() {
        let err = wrapped(rustls::Error::InvalidCertificate(
            CertificateError::NotValidForNameContext {
                expected: rustls::pki_types::ServerName::try_from("secret-host.internal")
                    .unwrap()
                    .to_owned(),
                presented: vec!["other-secret-host.internal".into()],
            },
        ));
        let oxi = certificate_rejection(err.as_ref()).expect("a rejection");
        let text = format!("{oxi} {oxi:?}");
        assert!(text.contains("NotValidForName"), "{text}");
        assert!(!text.contains("secret-host"), "{text}");
    }

    #[test]
    fn other_tls_errors_are_not_rejections() {
        let err = wrapped(rustls::Error::HandshakeNotComplete);
        assert!(certificate_rejection(err.as_ref()).is_none());
        let plain = io::Error::from(io::ErrorKind::ConnectionReset);
        assert!(certificate_rejection(&plain).is_none());
    }
}
