//! The kubeconfig entry for an AKS cluster: `kubelogin get-token` as an exec plugin.
//!
//! Only Azure AD clusters are supported: the entry authenticates with the signed-in `az`
//! account (`kubelogin --login azurecli`), so tokens are fetched at connect time and never
//! stored. A cluster with local accounts only would need a client certificate or static token
//! in the file, which an [`KubeconfigEntry`] cannot hold by design; it is refused with the
//! reason.

use oxikube_domain::cloud::{DiscoveredCluster, ExecInteractiveMode, ExecPlugin, KubeconfigEntry};
use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::CloudScope;

use super::KUBELOGIN_INSTALL_URL;
use super::credentials;
use super::model::{AadProfile, AksCluster, resource_group_of};
use crate::process::{Cli, CliInvocation, ProcessRunner, RunOpts};

/// The application id of AKS's managed Azure AD server, the `--server-id` of every managed
/// AAD cluster (as `kubelogin convert-kubeconfig` writes it).
pub(super) const MANAGED_AAD_SERVER_ID: &str = "6dae42f8-4368-4678-94ff-3960e28e3630";

/// The context name of a cluster: `aks:<subscription-id>:<resource-group>:<name>`, stable so
/// merging the same cluster twice replaces its entry.
pub(super) fn context_name(subscription: &str, resource_group: &str, name: &str) -> ContextName {
    ContextName::new(format!("aks:{subscription}:{resource_group}:{name}"))
}

/// The `kubelogin` arguments for an AAD cluster.
fn kubelogin_args(profile: &AadProfile) -> Vec<String> {
    let server_id = profile
        .server_app_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(MANAGED_AAD_SERVER_ID);
    let mut args = vec![
        "get-token".to_owned(),
        "--login".to_owned(),
        "azurecli".to_owned(),
        "--server-id".to_owned(),
        server_id.to_owned(),
    ];
    if let Some(tenant) = profile.tenant_id.as_deref().filter(|t| !t.is_empty()) {
        args.push("--tenant-id".to_owned());
        args.push(tenant.to_owned());
    }
    args
}

/// Builds the entry for `cluster`. See the module docs.
pub(super) async fn build(
    // `'static`: `run_json` is implemented on `dyn ProcessRunner + 'static` (E18-S01).
    runner: &(dyn ProcessRunner + 'static),
    cluster: &DiscoveredCluster,
) -> OxiResult<KubeconfigEntry> {
    let CloudScope::Azure {
        subscription_id, ..
    } = &cluster.scope
    else {
        return Err(OxiError::validation(format!(
            "the Azure adapter cannot connect to {} cluster `{}`",
            cluster.provider(),
            cluster.name
        )));
    };
    let group = cluster
        .native_id
        .as_deref()
        .and_then(resource_group_of)
        .ok_or_else(|| {
            OxiError::validation(format!(
                "AKS cluster `{}` has no resource id, so its resource group is unknown",
                cluster.name
            ))
        })?;
    let invocation = CliInvocation::new(
        Cli::Az,
        [
            "aks",
            "show",
            "--name",
            &cluster.name,
            "--resource-group",
            group,
            "--subscription",
            subscription_id,
        ],
    )
    .with_opts(RunOpts::default());
    let shown: AksCluster = runner.run_json(invocation).await?;
    if shown.is_stopped() {
        return Err(OxiError::unsupported(format!(
            "AKS cluster `{}` is stopped; start it with `az aks start --name {} \
             --resource-group {group}`",
            cluster.name, cluster.name
        )));
    }
    let Some(profile) = shown.aad_profile else {
        return Err(OxiError::unsupported(format!(
            "AKS cluster `{}` signs in with local accounts only, which need a stored \
             certificate or token; Oxikube connects with Azure AD (exec) sign-in only. Enable \
             Azure AD integration on the cluster",
            cluster.name
        )));
    };
    let kubelogin = runner.locate(Cli::Kubelogin).await.ok_or_else(|| {
        OxiError::unsupported(format!(
            "AKS cluster `{}` uses Azure AD sign-in, which needs `kubelogin`: install it \
             (`az aks install-cli`, or {KUBELOGIN_INSTALL_URL}) or set `{}` in settings",
            cluster.name,
            Cli::Kubelogin.setting_key()
        ))
    })?;
    let (server, ca_data) = match (&cluster.endpoint, &cluster.ca_data) {
        (Some(server), Some(ca)) => (server.clone(), Some(ca.clone())),
        _ => {
            let info =
                credentials::fetch_cluster_info(runner, subscription_id, group, &cluster.name)
                    .await?;
            (
                cluster.endpoint.clone().unwrap_or(info.server),
                cluster.ca_data.clone().or(info.ca_data),
            )
        }
    };
    let mut exec = ExecPlugin::new(
        kubelogin.to_string_lossy(),
        kubelogin_args(&profile),
        ExecInteractiveMode::Never,
    );
    exec.install_hint = Some(format!("Install kubelogin: {KUBELOGIN_INSTALL_URL}"));
    Ok(KubeconfigEntry {
        context: context_name(subscription_id, group, &cluster.name),
        server,
        ca_data,
        exec,
    })
}
