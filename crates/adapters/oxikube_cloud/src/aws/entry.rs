//! The exec-auth kubeconfig entry for an EKS cluster.

use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    CloudScope, DiscoveredCluster, ExecInteractiveMode, ExecPlugin, KubeconfigEntry,
};

use super::DEFAULT_PROFILE;

/// The context name for an EKS cluster: deterministic and collision free across profiles and
/// regions, so the managed kubeconfig merges idempotently.
pub(super) fn context_name(profile: &str, region: &str, cluster: &str) -> String {
    format!("eks:{profile}:{region}:{cluster}")
}

/// `aws eks get-token` for `cluster`. The token is fetched at connect time; none is stored.
pub(super) fn kubeconfig_entry(cluster: &DiscoveredCluster) -> OxiResult<KubeconfigEntry> {
    let CloudScope::Aws { profile, region } = &cluster.scope else {
        return Err(OxiError::validation(format!(
            "{} cluster `{}` is not an EKS cluster",
            cluster.provider(),
            cluster.name
        )));
    };
    let server = cluster.endpoint.clone().ok_or_else(|| {
        OxiError::validation(format!(
            "EKS cluster `{}` has no endpoint yet (status {})",
            cluster.name, cluster.status
        ))
    })?;
    let mut args: Vec<String> = ["eks", "get-token", "--cluster-name", &cluster.name]
        .map(str::to_owned)
        .into();
    args.extend(["--region".to_owned(), region.clone()]);
    if profile != DEFAULT_PROFILE {
        args.extend(["--profile".to_owned(), profile.clone()]);
    }
    let mut exec = ExecPlugin::new("aws", args, ExecInteractiveMode::IfAvailable);
    exec.install_hint = Some(
        "Install the AWS CLI v2: https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html"
            .to_owned(),
    );
    Ok(KubeconfigEntry {
        context: ContextName::new(context_name(profile, region, &cluster.name)),
        server,
        ca_data: cluster.ca_data.clone(),
        exec,
    })
}
