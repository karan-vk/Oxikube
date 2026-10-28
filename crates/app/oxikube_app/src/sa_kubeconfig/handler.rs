//! The `serviceaccount::GenerateKubeconfig` handler.

use std::sync::Arc;

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::secret::SecretText;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClusterEndpoint, ClusterSourcePort, ResourceReader, Subresource};
use serde_json::json;

use super::build::{KubeconfigSpec, Tokens};
use super::spec::format_lifetime;
use super::token::{self, IssuedToken};
use super::vault::{GeneratedKubeconfig, KubeconfigVault, TokenSource};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::guard::Mutation;
use crate::session::ClusterSessionManager;

/// The warning shown with, and written into, a kubeconfig whose token never expires.
pub const LONG_LIVED_WARNING: &str = "WARNING: this token comes from a service-account-token Secret. It does not expire \
     and stays valid until that Secret or the ServiceAccount is deleted.";

/// Registers `serviceaccount::GenerateKubeconfig` on `registry` (tool stub
/// `k8s.serviceaccount_generate_kubeconfig`).
///
/// The guard runs first: refused on a read-only cluster, one confirmation, an audit record naming
/// the account and the lifetime. The handler reads the account, reads the cluster's server
/// address and CA through `sources`, asks for a `TokenRequest` through the guard's writer and
/// keeps the finished kubeconfig in `vault`. Output `data` is `{"outcome", "source",
/// "expires_at", "context"}`: never the token.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
    sources: Arc<dyn ClusterSourcePort>,
    vault: KubeconfigVault,
) -> Result<(), RegisterError> {
    let id = CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG;
    let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let (sessions, sources, vault) = (sessions.clone(), sources.clone(), vault.clone());
        async move {
            let mutation = cx.require_mutation()?;
            run(command, mutation, &sessions, sources.as_ref(), &vault).await
        }
    })
}

async fn run(
    command: Command,
    mutation: &Mutation,
    sessions: &ClusterSessionManager,
    sources: &dyn ClusterSourcePort,
    vault: &KubeconfigVault,
) -> OxiResult<CommandOutput> {
    let Command::ServiceAccountGenerateKubeconfig {
        target,
        expiration_seconds,
        audiences,
        allow_legacy_token,
    } = command
    else {
        return Err(OxiError::validation(
            "not a serviceaccount::GenerateKubeconfig command",
        ));
    };
    if !target.gvk.group.is_empty() || &*target.gvk.kind != "ServiceAccount" {
        return Err(OxiError::validation(format!(
            "a kubeconfig is made for a ServiceAccount, not a {}",
            target.gvk.kind
        )));
    }
    let namespace = target
        .namespace()
        .ok_or_else(|| OxiError::validation("a ServiceAccount is namespaced"))?
        .to_owned();
    let seconds = token::checked_expiration(expiration_seconds)?;
    let reader: Arc<dyn ResourceReader> = sessions
        .get(&target.cluster)
        .and_then(|session| session.resources())
        .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;
    // The account must exist: a missing one is `NotFound`, and a later `NotFound` from the token
    // call then means the cluster lacks TokenRequest.
    reader
        .get(&target.gvk, Some(&namespace), &target.name)
        .await?;
    let names = ClusterNames::of(&target.cluster, sources, sessions).await;
    let endpoint = sources.cluster_endpoint(&target.cluster).await?;

    if mutation.dry_run() {
        return Ok(CommandOutput {
            message: Some(format!(
                "Dry run: a token for {namespace}/{} would be requested for {}",
                target.name,
                format_lifetime(seconds)
            )),
            data: Some(json!({"outcome": "dry_run", "context": names.context})),
        });
    }

    let issued = issue(
        &target,
        &namespace,
        seconds,
        &audiences,
        allow_legacy_token,
        mutation,
        reader.as_ref(),
    )
    .await?;
    let (token, source, expires_at) = issued;
    let warning = source
        .is_long_lived()
        .then(|| LONG_LIVED_WARNING.to_owned());
    let spec = spec_of(
        &target,
        &namespace,
        &names,
        endpoint,
        &token,
        warning.as_deref(),
        expires_at.as_deref(),
    );
    let text = spec.render(Tokens::Revealed).map_err(OxiError::internal)?;
    let preview = spec.render(Tokens::Masked).map_err(OxiError::internal)?;
    let context = spec.context_name();
    vault.put(GeneratedKubeconfig {
        service_account: target.clone(),
        text: SecretText::new(text),
        preview,
        source,
        expires_at: expires_at.clone(),
        warning,
    });
    // The data carries the facts of the result and never the token or the file.
    tracing::info!(
        cluster = %target.cluster,
        namespace,
        service_account = %target.name,
        source = source.label(),
        "kubeconfig generated"
    );
    Ok(CommandOutput {
        message: Some(match &expires_at {
            Some(when) => format!(
                "Kubeconfig for {namespace}/{} is ready (expires {when}): open the ServiceAccount to copy or save it",
                target.name
            ),
            None => format!(
                "Kubeconfig for {namespace}/{} is ready (the token does not expire): open the ServiceAccount to copy or save it",
                target.name
            ),
        }),
        data: Some(json!({
            "outcome": "generated",
            "source": source.label(),
            "expires_at": expires_at,
            "context": context,
        })),
    })
}

/// The token: a `TokenRequest`, or on a cluster without one a legacy Secret.
async fn issue(
    target: &ResourceRef,
    namespace: &str,
    seconds: u32,
    audiences: &[String],
    allow_legacy_token: bool,
    mutation: &Mutation,
    reader: &dyn ResourceReader,
) -> OxiResult<(SecretText, TokenSource, Option<String>)> {
    let body = token::token_request_body(seconds, audiences);
    let requested = mutation
        .writer()
        .create_subresource(
            &target.gvk,
            Some(namespace),
            &target.name,
            &Subresource::Other("token".into()),
            &body,
            &mutation.write_options(),
        )
        .await;
    match requested {
        Ok(response) => {
            let IssuedToken { token, expires_at } = token::parse_issued(&response)?;
            Ok((token, TokenSource::TokenRequest, expires_at))
        }
        Err(error) if token::lacks_token_request(&error) => {
            legacy(target, namespace, allow_legacy_token, mutation, reader).await
        }
        Err(error) => Err(error),
    }
}

/// The fallback for a cluster without `TokenRequest`: an existing legacy Secret, else (with
/// consent) a new one.
async fn legacy(
    target: &ResourceRef,
    namespace: &str,
    allow_legacy_token: bool,
    mutation: &Mutation,
    reader: &dyn ResourceReader,
) -> OxiResult<(SecretText, TokenSource, Option<String>)> {
    let refused = || {
        OxiError::validation(
            "this cluster has no TokenRequest API, so the only token is a long-lived \
             service-account-token Secret. Allow the legacy token to read or create one.",
        )
    };
    if !allow_legacy_token {
        return Err(refused());
    }
    if let Some(token) = token::existing_legacy_token(reader, namespace, &target.name).await? {
        return Ok((token, TokenSource::LegacySecret, None));
    }
    let token = token::create_legacy_token(
        reader,
        mutation.writer(),
        namespace,
        &target.name,
        &mutation.write_options(),
    )
    .await?;
    Ok((token, TokenSource::CreatedLegacySecret, None))
}

/// How the source cluster is named in the generated file.
struct ClusterNames {
    /// The kubeconfig `cluster` entry's name.
    cluster: String,
    /// The source context's name, for the output.
    context: String,
}

impl ClusterNames {
    async fn of(
        cluster: &ClusterId,
        sources: &dyn ClusterSourcePort,
        sessions: &ClusterSessionManager,
    ) -> Self {
        let from_catalog = sources.contexts().await.ok().and_then(|contexts| {
            contexts
                .into_iter()
                .find(|context| &context.cluster == cluster)
        });
        let context = from_catalog
            .as_ref()
            .map(|entry| entry.context.to_string())
            .or_else(|| sessions.get(cluster).map(|s| s.context().to_string()))
            .unwrap_or_else(|| "cluster".to_owned());
        let cluster = from_catalog
            .and_then(|entry| entry.cluster_name)
            .unwrap_or_else(|| context.clone());
        Self { cluster, context }
    }
}

fn spec_of(
    target: &ResourceRef,
    namespace: &str,
    names: &ClusterNames,
    endpoint: ClusterEndpoint,
    token: &SecretText,
    warning: Option<&str>,
    expires_at: Option<&str>,
) -> KubeconfigSpec {
    let mut notes = vec![format!(
        "Kubeconfig for ServiceAccount {namespace}/{} (generated by Oxikube).",
        target.name
    )];
    if let Some(when) = expires_at {
        notes.push(format!("The token expires at {when}."));
    }
    if let Some(warning) = warning {
        notes.push(warning.to_owned());
    }
    KubeconfigSpec {
        cluster_name: names.cluster.clone(),
        server: endpoint.server,
        certificate_authority_pem: endpoint.certificate_authority_pem,
        insecure_skip_tls_verify: endpoint.insecure_skip_tls_verify,
        tls_server_name: endpoint.tls_server_name,
        service_account: target.name.to_string(),
        namespace: namespace.to_owned(),
        token: token.clone(),
        notes,
    }
}
