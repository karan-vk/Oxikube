//! The `helm::Upgrade` handler (tool stub `helm.upgrade`) and `helm::UpgradeRelease` (tool stub
//! `helm.upgrade_release`, opens the tab): the preview's dry run and the upgrade with its output
//! published line by line.

use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::helm::{InstallRequest, ManifestSummary, ReleaseRef, ResourceChanges, Values};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmPort;
use serde_json::json;

use super::requests::{UpgradeRequests, UpgradeTarget};
use super::spec::UpgradeSpec;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::helm_install::{ReleaseProgress, mask_secrets};

/// Registers `helm::Upgrade` and `helm::UpgradeRelease` on `registry`. The binary installs it as
/// `oxikube_app::helm_upgrade`.
///
/// `helm::Upgrade` is a medium-risk mutation: refused on a read-only cluster, one simple
/// confirmation, an audit record naming the release, the target chart, `reuse_values` and (from
/// the handler) the chart it ran, never the values. A dry-run dispatch is the preview: `helm
/// template` renders the new chart, `helm upgrade --dry-run` has the server validate it, and the
/// objects are compared with the release's current manifest; nothing is upgraded. Its output's
/// `data` is `{"outcome": "dry_run", "manifest", "server_error", "from_chart", "changes"}`. An
/// upgrade's is `{"outcome": "upgraded", "release", "namespace"}`; a dispatch dropped while it
/// runs kills `helm` (the guard audits it as cancelled).
///
/// # Errors
///
/// A [`RegisterError`] when a command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    helm: Arc<dyn HelmPort>,
    progress: ReleaseProgress,
    requests: UpgradeRequests,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::HELM_UPGRADE)
        .ok_or(RegisterError::Undeclared(CommandId::HELM_UPGRADE))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let helm = helm.clone();
        let progress = progress.clone();
        async move {
            let spec = UpgradeSpec::of(&command)
                .ok_or_else(|| OxiError::validation("not a helm::Upgrade"))?;
            let dry_run = cx.require_mutation()?.dry_run();
            let request = spec.request(dry_run)?;
            if dry_run {
                preview(helm.as_ref(), &spec, request, &cx).await
            } else {
                upgrade(helm.as_ref(), &spec, request, &cx, &progress).await
            }
        }
    })?;
    let meta = *command::lookup(CommandId::HELM_UPGRADE_RELEASE)
        .ok_or(RegisterError::Undeclared(CommandId::HELM_UPGRADE_RELEASE))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let requests = requests.clone();
        async move {
            let Command::HelmUpgradeRelease { namespace, release } = command else {
                return Err(OxiError::validation("not a helm::UpgradeRelease"));
            };
            let release = namespace.zip(release);
            let message = match &release {
                Some((namespace, name)) => format!("Upgrade {namespace}/{name}"),
                None => "Upgrade a release".to_owned(),
            };
            requests.request(UpgradeTarget {
                release,
                chart: None,
            });
            Ok(CommandOutput {
                message: Some(message),
                data: None,
            })
        }
    })
}

/// The chart a release runs now, as `name-version`, and its revision (`0` when unknown).
async fn current_chart(helm: &dyn HelmPort, release: &ReleaseRef) -> (String, u32) {
    match helm.history(release).await {
        Ok(revisions) => revisions
            .first()
            .map(|r| (r.chart.to_string(), r.revision))
            .unwrap_or_default(),
        Err(_) => (String::new(), 0),
    }
}

/// The effective values of the rehearsal render: with `--reuse-values` the release's computed
/// values (the previous revision's) under the edits, else the edits alone.
async fn effective_values(
    helm: &dyn HelmPort,
    spec: &UpgradeSpec,
    edits: Values,
) -> OxiResult<Values> {
    if !spec.reuse_values {
        return Ok(edits);
    }
    let previous = helm.values(&spec.release, None, true).await?;
    Ok(Values::merge(&previous, &edits))
}

/// Renders and validates the upgrade (a dry run) and compares it with the current manifest.
async fn preview(
    helm: &dyn HelmPort,
    spec: &UpgradeSpec,
    request: oxikube_domain::helm::UpgradeRequest,
    cx: &HandlerContext,
) -> OxiResult<CommandOutput> {
    let mut render = InstallRequest::new(
        spec.cluster().clone(),
        &spec.release.name,
        &spec.release.namespace,
        spec.chart.clone(),
    );
    render.values = effective_values(helm, spec, request.values.clone()).await?;
    render.wait = false;
    let server_check = async {
        let mut output = helm.upgrade(&request).await?;
        while let Some(line) = output.next().await {
            line?;
        }
        Ok::<_, OxiError>(())
    };
    let (manifest, server, current, from) = futures::join!(
        helm.template(&render),
        server_check,
        helm.manifest(&spec.release, None),
        current_chart(helm, &spec.release),
    );
    let rendered = manifest?;
    let current = current?;
    // Parsing is linear in the manifest and runs on the Tokio bridge, never on the UI thread.
    let changes = ResourceChanges::between(&current, &rendered);
    let manifest = mask_secrets(&rendered);
    let server_error = server.err().map(|error| error.message().to_owned());
    if server_error.is_some() {
        cx.note_audit(format!("from={} server_dry_run=refused", from.0));
    } else {
        cx.note_audit(format!("from={}", from.0));
    }
    let summary = ManifestSummary::of(&manifest);
    Ok(CommandOutput {
        message: Some(format!(
            "Dry run of the upgrade of {}: {}",
            spec.release.name,
            changes.describe()
        )),
        data: Some(json!({
            "outcome": "dry_run",
            "manifest": manifest,
            "objects": summary.describe(),
            "server_error": server_error,
            "from_chart": from.0,
            "changes": {
                "added": changes.added.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "removed": changes.removed.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "changed": changes.changed.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "unchanged": changes.unchanged,
            },
        })),
    })
}

/// Upgrades, publishing each line of `helm`'s output to the release's followers.
async fn upgrade(
    helm: &dyn HelmPort,
    spec: &UpgradeSpec,
    request: oxikube_domain::helm::UpgradeRequest,
    cx: &HandlerContext,
    progress: &ReleaseProgress,
) -> OxiResult<CommandOutput> {
    let publisher = progress.start(&spec.release)?;
    let (from, revision) = current_chart(helm, &spec.release).await;
    cx.note_audit(format!("from={from} from_revision={revision}"));
    let mut output = helm.upgrade(&request).await?;
    while let Some(line) = output.next().await {
        publisher.publish(&line?);
    }
    Ok(CommandOutput {
        message: Some(format!(
            "Upgraded release {} in namespace {}",
            spec.release.name, spec.release.namespace
        )),
        data: Some(json!({
            "outcome": "upgraded",
            "release": spec.release.name,
            "namespace": spec.release.namespace,
        })),
    })
}
