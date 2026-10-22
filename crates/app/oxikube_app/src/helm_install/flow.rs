//! [`InstallFlow`]: the install tab's side of `helm::Install`: preview, ask, install, follow.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::channel::mpsc;
use futures::future::BoxFuture;
use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::helm::{ManifestSummary, OutputLine};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::safety::ConfirmTier;
use oxikube_ports::ListOptions;
use serde_json::Value;

use super::progress::ReleaseProgress;
use super::spec::{InstallError, InstallPreview, InstallSpec};
use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::guard::{Confirmation, ConfirmationRequest};
use crate::{ClusterSessionManager, CommandBus};

/// Namespaces are listed this many at a time.
const PAGE: u32 = 500;

/// An install that was confirmed: its output as it arrives and its end.
pub struct InstallStart {
    /// Every line of `helm`'s output, in order; ends when the install ends.
    pub lines: mpsc::UnboundedReceiver<OutputLine>,
    /// The dispatch: the handler's message when the release was installed, else why not. Run it
    /// on the Tokio bridge; **dropping it cancels the install** (`helm` is killed; the guard
    /// audits it as cancelled).
    pub run: BoxFuture<'static, Result<String, InstallError>>,
}

/// Previews, confirms and runs installs for the install tab, every step a `helm::Install`
/// dispatch through the guard. Cheap to clone. Plain async: the tab runs the futures on the
/// Tokio bridge (`spawn_kube`).
#[derive(Clone)]
pub struct InstallFlow {
    bus: CommandBus,
    sessions: ClusterSessionManager,
    progress: ReleaseProgress,
    who: Arc<str>,
}

impl std::fmt::Debug for InstallFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallFlow").finish_non_exhaustive()
    }
}

impl InstallFlow {
    /// A flow dispatching on `bus` as `who` (the local user, for the audit log), following
    /// installs on `progress` (the handler's, see
    /// [`register_commands`](super::register_commands)).
    pub fn new(
        bus: CommandBus,
        sessions: ClusterSessionManager,
        progress: ReleaseProgress,
        who: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            bus,
            sessions,
            progress,
            who: who.into(),
        }
    }

    /// The names of the cluster's namespaces, sorted, for the namespace field.
    ///
    /// # Errors
    ///
    /// The read's error (forbidden, not connected).
    pub async fn namespaces(&self, cluster: &ClusterId) -> Result<Vec<String>, OxiError> {
        let session = self
            .sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        let reader = session
            .resources()
            .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;
        let mut options = ListOptions::default().limit(PAGE);
        let mut names = Vec::new();
        loop {
            let page = reader.list(&Gvk::namespace(), None, &options).await?;
            names.extend(page.items.iter().map(|ns| ns.name().to_owned()));
            match page.continue_token.filter(|token| !token.is_empty()) {
                Some(token) => options = options.continue_from(token),
                None => break,
            }
        }
        names.sort();
        Ok(names)
    }

    /// The preview of `spec`: a dry-run `helm::Install` through the guard (`helm template` for
    /// the manifest, the server's dry run for its verdict). Nothing is installed. The guard's
    /// confirmation of the rehearsal itself is answered here: the user asked for it and it changes
    /// nothing; the dry run is audited as one.
    ///
    /// # Errors
    ///
    /// [`InstallError::Invalid`] when the values are not a YAML map (nothing sent),
    /// [`InstallError::ReadOnly`] on a read-only cluster, [`InstallError::Failed`] when the chart
    /// does not render.
    pub async fn preview(&self, spec: InstallSpec) -> Result<InstallPreview, InstallError> {
        spec.values
            .parse()
            .map_err(|error| InstallError::Invalid(error.to_string()))?;
        let command = spec.command();
        let context = self.context(&spec.cluster, true);
        let outcome = match self.bus.dispatch(command.clone(), context.clone()).await {
            Ok(Outcome::NeedsConfirmation(asked)) if asked.tier <= ConfirmTier::Simple => {
                let answer = Confirmation::simple(asked.token);
                self.bus
                    .dispatch(command, context.with_confirmation(answer))
                    .await
            }
            Ok(Outcome::NeedsConfirmation(asked)) => {
                let _ = self.bus.decline(asked.token).await;
                return Err(InstallError::Unconfirmed);
            }
            other => other,
        };
        let output = match outcome {
            Ok(Outcome::Completed(output)) => output,
            Ok(Outcome::NeedsConfirmation(_)) => return Err(InstallError::Unconfirmed),
            Err(error) => return Err(classify(error)),
        };
        let data = output.data.unwrap_or(Value::Null);
        let manifest: Arc<str> = data
            .get("manifest")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into();
        Ok(InstallPreview {
            summary: ManifestSummary::of(&manifest),
            manifest,
            server_error: data
                .get("server_error")
                .and_then(Value::as_str)
                .map(str::to_owned),
            spec,
        })
    }

    /// Asks the guard to install what `preview` showed: the answer is its confirmation request
    /// (tier simple, risk low), for the tab to show. Nothing runs until
    /// [`start`](Self::start) answers it, or [`decline`](Self::decline) drops it.
    ///
    /// # Errors
    ///
    /// [`InstallError::Invalid`] when the preview's server dry run refused the install,
    /// [`InstallError::ReadOnly`] on a read-only cluster (audited as denied).
    pub async fn ask(&self, preview: &InstallPreview) -> Result<ConfirmationRequest, InstallError> {
        if let Some(why) = &preview.server_error {
            return Err(InstallError::Invalid(why.clone()));
        }
        let context = self.context(&preview.spec.cluster, false);
        match self.bus.dispatch(preview.spec.command(), context).await {
            Ok(Outcome::NeedsConfirmation(request)) => Ok(request),
            // A mutation at risk low always asks; anything else ran without being confirmed.
            Ok(Outcome::Completed(_)) => Err(InstallError::Failed(
                "the install ran without a confirmation".to_owned(),
            )),
            Err(error) => Err(classify(error)),
        }
    }

    /// Installs `preview`'s spec, answering `confirmation` (from [`ask`](Self::ask)): follows the
    /// release first, then dispatches with the answer.
    pub fn start(
        &self,
        preview: &InstallPreview,
        confirmation: &ConfirmationRequest,
    ) -> InstallStart {
        let spec = preview.spec.clone();
        let lines = self.progress.subscribe(&spec.release_ref());
        let bus = self.bus.clone();
        let progress = self.progress.clone();
        let context = self
            .context(&spec.cluster, false)
            .with_confirmation(Confirmation::simple(confirmation.token));
        let run = async move {
            let result = bus.dispatch(spec.command(), context).await;
            progress.forget(&spec.release_ref());
            match result {
                Ok(Outcome::Completed(output)) => Ok(output.message.unwrap_or_default()),
                Ok(Outcome::NeedsConfirmation(_)) => Err(InstallError::Unconfirmed),
                Err(error) => Err(classify(error)),
            }
        };
        InstallStart {
            lines,
            run: run.boxed(),
        }
    }

    /// Declines `confirmation`: nothing is installed and the guard audits it as cancelled.
    pub async fn decline(&self, confirmation: ConfirmationRequest) {
        if let Err(error) = self.bus.decline(confirmation.token).await {
            tracing::debug!(%error, "declining an install confirmation");
        }
    }

    fn context(&self, cluster: &ClusterId, dry_run: bool) -> DispatchContext {
        DispatchContext::new(Initiator::Ui, self.who.clone())
            .with_cluster(cluster.clone())
            .with_dry_run(dry_run)
    }
}

/// The [`InstallError`] of a failed dispatch.
fn classify(error: DispatchError) -> InstallError {
    match error {
        DispatchError::ReadOnly { context, .. } => InstallError::ReadOnly {
            context: context.to_string(),
        },
        DispatchError::Confirmation(_) => InstallError::Unconfirmed,
        other => InstallError::Failed(OxiError::from(other).message().to_owned()),
    }
}
