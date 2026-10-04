//! TLS settings of a context: insecure mode, custom CAs, `tls-server-name`.
//!
//! kube resolves these from the kubeconfig into [`Config`] fields
//! (`Config::from_custom_kubeconfig`); this module adds what kube leaves implicit and
//! makes the risky parts visible.
//!
//! | kubeconfig cluster field | [`Config`] field | note |
//! |---|---|---|
//! | `insecure-skip-tls-verify: true` | `accept_invalid_certs` | the only way verification is disabled; see below |
//! | `certificate-authority-data` (base64 PEM) | `root_cert` | overrides `certificate-authority`, like kubectl |
//! | `certificate-authority` (file) | `root_cert` + `root_cert_file` | the file is re-read about every 60 s, so a rotated CA needs no restart |
//! | `tls-server-name` | `tls_server_name` | SNI and certificate name for servers reached by IP or through a tunnel |
//!
//! # Insecure mode
//!
//! `accept_invalid_certs` is assigned from the kubeconfig and from nothing else: no
//! setting, environment variable or fallback enables it. Each build of an insecure
//! context logs one `warn` naming the context and the server host (no credentials, no
//! certificate data), and [`ClientPool::tls_verification_disabled`](super::ClientPool::tls_verification_disabled)
//! lets the session show a badge. A verified connection logs nothing and reports `false`.
//!
//! # Custom CAs
//!
//! * An empty `certificate-authority-data` or `certificate-authority` counts as unset (as
//!   in client-go); kube would otherwise build an empty trust store that rejects every
//!   server.
//! * A CA that decodes but holds no PEM `CERTIFICATE` block is a
//!   [`Validation`](oxikube_domain::ErrorKind::Validation) error naming the certificate
//!   authority; so are undecodable base64, malformed PEM and unreadable files
//!   (`auth::classify_kubeconfig`) and certificates rustls rejects (`auth::classify_tls_setup`).
//! * The reloading file is only wired for an absolute path and a verifying connection.
//!   Relative paths (a pasted kubeconfig) are read once, like kube does. When both forms
//!   are present, the inline data wins and nothing is reloaded.
//! * CA parsing happens once per client build, not per request.
//!
//! # TLS provider
//!
//! The workspace enables kube's `rustls-tls` with `ring` and nothing else. Do not add
//! `openssl-tls`, or a second rustls provider such as `aws-lc-rs`: mixed providers panic
//! at runtime when rustls cannot pick a default. `cargo tree -p oxikube_kube -i openssl`
//! and `-i aws-lc-rs` must stay empty (checked in the E03-S07 PR).

use std::path::PathBuf;

use kube::Config;
use kube::config::Kubeconfig;
use oxikube_domain::OxiError;
use tracing::warn;

use super::entry::ContextDefinition;

/// Drops empty CA fields from the cluster entries before kube reads them.
///
/// kube treats `certificate-authority-data: ""` as a CA with no certificates, which
/// leaves an empty trust store. client-go treats it as unset.
pub(super) fn normalize(kubeconfig: &mut Kubeconfig) {
    for named in &mut kubeconfig.clusters {
        let Some(cluster) = named.cluster.as_mut() else {
            continue;
        };
        if cluster
            .certificate_authority_data
            .as_deref()
            .is_some_and(str::is_empty)
        {
            cluster.certificate_authority_data = None;
        }
        if cluster
            .certificate_authority
            .as_deref()
            .is_some_and(str::is_empty)
        {
            cluster.certificate_authority = None;
        }
    }
}

/// Applies the TLS rules to the [`Config`] kube resolved for `definition`.
pub(super) fn apply(config: &mut Config, definition: &ContextDefinition) -> Result<(), OxiError> {
    let Some(cluster) = definition.cluster() else {
        return Ok(());
    };
    let context = definition.context();

    // The single path that disables verification: the kubeconfig says so.
    config.accept_invalid_certs = definition.tls_verification_disabled();
    if config.accept_invalid_certs {
        warn!(
            context = %context,
            server_host = ?definition.server_host(),
            "TLS certificate verification is disabled for this context \
             (insecure-skip-tls-verify in the kubeconfig)"
        );
    }

    let has_ca =
        cluster.certificate_authority_data.is_some() || cluster.certificate_authority.is_some();
    if has_ca && config.root_cert.as_ref().is_some_and(Vec::is_empty) {
        return Err(OxiError::validation(format!(
            "context `{context}`: the certificate authority in the kubeconfig contains no PEM \
             certificates"
        )));
    }

    // File-only CA: let kube re-read the file so a rotated CA is picked up.
    if !config.accept_invalid_certs
        && cluster.certificate_authority_data.is_none()
        && let Some(path) = cluster.certificate_authority.as_deref().map(PathBuf::from)
        && path.is_absolute()
    {
        config.root_cert_file = Some(path);
    }
    Ok(())
}
