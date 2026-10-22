//! The `helm::Install` handler (tool stub `helm.install`): the preview's dry run and the install
//! with its output published line by line.

use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::helm::{InstallRequest, ManifestSummary};
use oxikube_domain::{OxiError, OxiResult, redact};
use oxikube_ports::HelmPort;
use serde_json::json;

use super::progress::ReleaseProgress;
use super::spec::InstallSpec;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers `helm::Install` on `registry`, installing through `helm` and publishing each output
/// line on `progress`. The binary installs it as `oxikube_app::helm_install`.
///
/// The guard runs first: refused on a read-only cluster, one simple confirmation (risk low), and
/// an audit record naming the release, the chart and the flags, never the values. A dry-run
/// dispatch is the preview: `helm template` renders the chart and `helm install --dry-run` has
/// the server validate it; nothing is installed. Its output's `data` is
/// `{"outcome": "dry_run", "manifest", "server_error"}` (the manifest with Secret data masked).
/// An install's is `{"outcome": "installed", "release", "namespace"}`; a dispatch dropped while
/// it runs kills `helm` (the guard audits it as cancelled).
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    helm: Arc<dyn HelmPort>,
    progress: ReleaseProgress,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::HELM_INSTALL)
        .ok_or(RegisterError::Undeclared(CommandId::HELM_INSTALL))?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let helm = helm.clone();
        let progress = progress.clone();
        async move {
            let spec = InstallSpec::of(&command)
                .ok_or_else(|| OxiError::validation("not a helm::Install"))?;
            let dry_run = cx.require_mutation()?.dry_run();
            let request = spec.request(dry_run)?;
            if dry_run {
                preview(helm.as_ref(), &request, &cx).await
            } else {
                install(helm.as_ref(), &spec, &request, &progress).await
            }
        }
    })
}

/// Renders and validates `request` (a dry run): the template and the server's verdict, run side by
/// side.
async fn preview(
    helm: &dyn HelmPort,
    request: &InstallRequest,
    cx: &HandlerContext,
) -> OxiResult<CommandOutput> {
    let mut render = request.clone();
    render.dry_run = false;
    let server_check = async {
        let mut output = helm.install(request).await?;
        while let Some(line) = output.next().await {
            line?;
        }
        Ok::<_, OxiError>(())
    };
    let (manifest, server) = futures::join!(helm.template(&render), server_check);
    let manifest = mask_secrets(&manifest?);
    let server_error = server.err().map(|error| error.message().to_owned());
    if server_error.is_some() {
        cx.note_audit("server_dry_run=refused");
    }
    let summary = ManifestSummary::of(&manifest);
    Ok(CommandOutput {
        message: Some(format!(
            "Dry run of {} renders {}",
            request.release,
            summary.describe()
        )),
        data: Some(json!({
            "outcome": "dry_run",
            "manifest": manifest,
            "server_error": server_error,
        })),
    })
}

/// Installs, publishing each line of `helm`'s output to the release's followers.
async fn install(
    helm: &dyn HelmPort,
    spec: &InstallSpec,
    request: &InstallRequest,
    progress: &ReleaseProgress,
) -> OxiResult<CommandOutput> {
    let publisher = progress.start(&spec.release_ref())?;
    let mut output = helm.install(request).await?;
    while let Some(line) = output.next().await {
        publisher.publish(&line?);
    }
    Ok(CommandOutput {
        message: Some(format!(
            "Installed release {} in namespace {}",
            spec.release, spec.namespace
        )),
        data: Some(json!({
            "outcome": "installed",
            "release": spec.release,
            "namespace": spec.namespace,
        })),
    })
}

/// `manifest` with the documents of kind `Secret` passed through the shared redaction (their
/// `data` / `stringData` values and secret-named fields masked). Other documents are untouched,
/// so a ConfigMap reads as it will be installed.
pub(crate) fn mask_secrets(manifest: &str) -> String {
    let mut out = String::with_capacity(manifest.len());
    for (i, document) in split_documents(manifest).enumerate() {
        if i > 0 {
            out.push_str("---");
        }
        if ManifestSummary::of(document).kinds.contains_key("Secret") {
            out.push_str(&redact::redact(document));
        } else {
            out.push_str(document);
        }
    }
    out
}

/// The documents of `manifest`, each starting right after a `---` line's dashes (so joining them
/// with `---` gives the text back).
fn split_documents(manifest: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(manifest);
    std::iter::from_fn(move || {
        let text = rest?;
        let mut search = 0;
        loop {
            match text[search..].find("---") {
                Some(at) => {
                    let at = search + at;
                    let line_start = at == 0 || text.as_bytes()[at - 1] == b'\n';
                    let after = &text[at + 3..];
                    let line_end = after.is_empty() || after.starts_with(['\n', ' ', '\r']);
                    if line_start && line_end && at > 0 {
                        rest = Some(after);
                        return Some(&text[..at]);
                    }
                    search = at + 3;
                }
                None => {
                    rest = None;
                    return Some(text);
                }
            }
        }
    })
}
