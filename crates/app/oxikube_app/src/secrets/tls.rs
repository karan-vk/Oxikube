//! What a `kubernetes.io/tls` Secret says about its certificate: subject, issuer, validity and
//! names, read from the PEM in `tls.crt`. The private key is never parsed here beyond checking
//! that a PEM block of the right kind is present ([`check_private_key`]).

use jiff::Timestamp;
use x509_parser::extensions::GeneralName;
use x509_parser::pem::Pem;

/// The first certificate of a PEM chain, summarised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsInfo {
    /// The subject (`CN=example.com, O=Example`).
    pub subject: String,
    /// The issuer.
    pub issuer: String,
    /// When the certificate becomes valid.
    pub not_before: Timestamp,
    /// When it expires.
    pub not_after: Timestamp,
    /// The DNS names in the subject alternative name extension.
    pub dns_names: Vec<String>,
    /// How many certificates the PEM holds (a chain has more than one).
    pub certificates: usize,
}

impl TlsInfo {
    /// Whether the certificate has expired at `now`.
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now > self.not_after
    }

    /// Whole days from `now` until it expires (negative once expired).
    pub fn days_left(&self, now: Timestamp) -> i64 {
        self.not_after.as_second().saturating_sub(now.as_second()) / 86_400
    }
}

/// Why a certificate or key PEM was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PemProblem {
    /// No PEM block of the expected kind.
    #[error("no PEM block found: expected `{0}`")]
    Missing(&'static str),
    /// A PEM block that does not decode.
    #[error("the PEM text is damaged: {0}")]
    Damaged(String),
    /// A block that is not a certificate where one is required.
    #[error("the {0} block is not a certificate")]
    NotACertificate(String),
    /// A certificate that does not parse as X.509.
    #[error("the certificate cannot be read: {0}")]
    BadCertificate(String),
}

/// Reads the certificates of `pem`.
///
/// # Errors
///
/// [`PemProblem`] when there is no `CERTIFICATE` block, the PEM does not decode or a certificate
/// is not X.509.
pub fn parse_certificates(pem: &str) -> Result<TlsInfo, PemProblem> {
    let mut first: Option<TlsInfo> = None;
    let mut count = 0usize;
    for block in Pem::iter_from_buffer(pem.as_bytes()) {
        let block = block.map_err(|error| PemProblem::Damaged(error.to_string()))?;
        if block.label != "CERTIFICATE" && block.label != "TRUSTED CERTIFICATE" {
            return Err(PemProblem::NotACertificate(block.label));
        }
        let cert = block
            .parse_x509()
            .map_err(|error| PemProblem::BadCertificate(error.to_string()))?;
        count += 1;
        if first.is_none() {
            let validity = cert.validity();
            let at = |seconds: i64| {
                Timestamp::from_second(seconds)
                    .map_err(|error| PemProblem::BadCertificate(error.to_string()))
            };
            let dns_names = cert
                .subject_alternative_name()
                .ok()
                .flatten()
                .map(|san| {
                    san.value
                        .general_names
                        .iter()
                        .filter_map(|name| match name {
                            GeneralName::DNSName(dns) => Some((*dns).to_owned()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            first = Some(TlsInfo {
                subject: cert.subject().to_string(),
                issuer: cert.issuer().to_string(),
                not_before: at(validity.not_before.timestamp())?,
                not_after: at(validity.not_after.timestamp())?,
                dns_names,
                certificates: 0,
            });
        }
    }
    let mut info = first.ok_or(PemProblem::Missing("CERTIFICATE"))?;
    info.certificates = count;
    Ok(info)
}

/// Checks that `pem` holds a private key block (`PRIVATE KEY`, `RSA PRIVATE KEY`,
/// `EC PRIVATE KEY`, ...) with a body. The key is not parsed further.
///
/// # Errors
///
/// [`PemProblem`] when there is none, or the PEM does not decode.
pub fn check_private_key(pem: &str) -> Result<(), PemProblem> {
    for block in Pem::iter_from_buffer(pem.as_bytes()) {
        let block = block.map_err(|error| PemProblem::Damaged(error.to_string()))?;
        if block.label.ends_with("PRIVATE KEY") {
            return if block.contents.is_empty() {
                Err(PemProblem::Damaged("the key is empty".into()))
            } else {
                Ok(())
            };
        }
    }
    Err(PemProblem::Missing("PRIVATE KEY"))
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Self-signed certificates for tests, made at run time (no key material in the repository).

    use rcgen::{CertificateParams, DnType, KeyPair};

    /// A PEM certificate for `names` valid for `days` from now, and its private key PEM.
    pub fn certificate(names: &[&str], days: i64) -> (String, String) {
        let key = KeyPair::generate().expect("a key");
        let mut params =
            CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
                .expect("params");
        params
            .distinguished_name
            .push(DnType::CommonName, names.first().copied().unwrap_or("test"));
        let now = rcgen::date_time_ymd(2026, 1, 1);
        params.not_before = now;
        params.not_after = now + std::time::Duration::from_secs((days * 86_400) as u64);
        let cert = params.self_signed(&key).expect("self-signed");
        (cert.pem(), key.serialize_pem())
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::certificate;
    use super::*;

    #[test]
    fn a_certificate_gives_its_subject_names_and_expiry() {
        let (cert, key) = certificate(&["web.example.com", "api.example.com"], 90);
        let info = parse_certificates(&cert).unwrap();
        assert!(info.subject.contains("web.example.com"), "{}", info.subject);
        assert_eq!(info.dns_names, ["web.example.com", "api.example.com"]);
        assert_eq!(info.certificates, 1);
        assert_eq!(
            info.not_after.as_second() - info.not_before.as_second(),
            90 * 86_400
        );
        let before = info.not_before;
        assert!(!info.is_expired(before));
        assert!(info.is_expired(info.not_after + jiff::SignedDuration::from_secs(1)));
        assert_eq!(info.days_left(before), 90);
        assert!(check_private_key(&key).is_ok());
    }

    #[test]
    fn a_chain_counts_its_certificates() {
        let (one, _) = certificate(&["a.example.com"], 30);
        let (two, _) = certificate(&["b.example.com"], 30);
        let info = parse_certificates(&format!("{one}\n{two}")).unwrap();
        assert_eq!(info.certificates, 2);
        assert!(info.subject.contains("a.example.com"));
    }

    #[test]
    fn bad_pem_is_refused_with_a_reason() {
        assert_eq!(
            parse_certificates("not pem at all"),
            Err(PemProblem::Missing("CERTIFICATE"))
        );
        let (_, key) = certificate(&["x.example.com"], 30);
        assert!(matches!(
            parse_certificates(&key),
            Err(PemProblem::NotACertificate(_))
        ));
        let broken = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
        assert!(matches!(
            parse_certificates(broken),
            Err(PemProblem::BadCertificate(_))
        ));
        assert_eq!(
            check_private_key("nothing"),
            Err(PemProblem::Missing("PRIVATE KEY"))
        );
        let (cert, _) = certificate(&["x.example.com"], 30);
        assert_eq!(
            check_private_key(&cert),
            Err(PemProblem::Missing("PRIVATE KEY"))
        );
    }
}
