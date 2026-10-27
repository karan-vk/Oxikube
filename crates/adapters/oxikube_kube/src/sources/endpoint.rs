//! [`ClusterEndpoint`] of a catalog entry: the public half of its kubeconfig `cluster` entry.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::ClusterEndpoint;

use crate::kubeconfig::LoadedKubeconfig;
use crate::pool::ContextDefinition;

/// What the kubeconfig says about the API server of `cluster`, with a CA file read inline.
///
/// # Errors
///
/// `NotFound` for a cluster not in `loaded`; `Validation` for an entry without a server, or a CA
/// (inline or file) that is not readable PEM. Messages name the context and the file, never
/// their text.
pub(super) async fn endpoint_of(
    loaded: &LoadedKubeconfig,
    cluster: &ClusterId,
) -> OxiResult<ClusterEndpoint> {
    let context = loaded
        .context_names()
        .find(|name| loaded.cluster_id(name).as_ref() == Some(cluster))
        .ok_or_else(|| OxiError::not_found("the catalog has no such cluster"))?;
    let definition = ContextDefinition::from_kubeconfig(&loaded.merged, &context)
        .map(|definition| definition.with_in_cluster(loaded.is_in_cluster(&context)))
        .ok_or_else(|| OxiError::not_found("the catalog has no such cluster"))?;
    let entry = definition
        .cluster()
        .ok_or_else(|| missing(&context, "names no cluster entry"))?;
    let server = entry
        .server
        .clone()
        .filter(|server| !server.is_empty())
        .ok_or_else(|| missing(&context, "names no server"))?;
    let pem = if let Some(data) = entry
        .certificate_authority_data
        .as_deref()
        .filter(|data| !data.is_empty())
    {
        let bytes = STANDARD.decode(data.trim()).map_err(|_| {
            missing(
                &context,
                "has a certificate-authority-data that is not base64",
            )
        })?;
        Some(into_pem(bytes, &context)?)
    } else if let Some(path) = entry
        .certificate_authority
        .as_deref()
        .filter(|path| !path.is_empty())
    {
        let bytes = tokio::fs::read(path).await.map_err(|error| {
            OxiError::validation(format!(
                "context `{context}`: cannot read the certificate authority file `{path}`: {}",
                error.kind()
            ))
        })?;
        Some(into_pem(bytes, &context)?)
    } else {
        None
    };
    Ok(ClusterEndpoint {
        server,
        certificate_authority_pem: pem,
        insecure_skip_tls_verify: definition.tls_verification_disabled(),
        tls_server_name: entry
            .tls_server_name
            .clone()
            .filter(|name| !name.is_empty()),
    })
}

fn into_pem(bytes: Vec<u8>, context: &ContextName) -> OxiResult<String> {
    let text = String::from_utf8(bytes)
        .map_err(|_| missing(context, "has a certificate authority that is not PEM text"))?;
    if !text.contains("-----BEGIN CERTIFICATE-----") {
        return Err(missing(
            context,
            "has a certificate authority without a PEM certificate",
        ));
    }
    Ok(text)
}

fn missing(context: &ContextName, what: &str) -> OxiError {
    OxiError::validation(format!("context `{context}` {what}"))
}
