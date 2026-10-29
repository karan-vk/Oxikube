//! [`CopyService`]: the copy flow's plain async side: which clusters and namespaces to offer,
//! connecting the target on demand, the target's dry run as a preview, and the copy itself, all
//! as `resource::CopyTo` dispatches on the command bus.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{ConflictReason, OxiError};
use oxikube_ports::ListOptions;
use serde_json::Value;

use super::refusal::{Refusal, validate_name};
use super::types::{
    CopyChoice, CopyError, CopyOutcome, CopyPreview, CopyRequest, MissingReference,
};
use crate::apply::FieldConflict;
use crate::command_bus::{CommandOutput, DispatchContext, DispatchError, Outcome};
use crate::guard::{ConfirmationProvider, ConfirmationRequest, Decision};
use crate::namespaces::NamespaceCreator;
use crate::{ClusterCatalog, ClusterSessionManager, CommandBus};

/// The most pages of namespaces read for the picker; a cluster has hundreds, not thousands.
const MAX_NAMESPACE_PAGES: usize = 20;

/// A cluster the copy can go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterChoice {
    /// The cluster.
    pub id: ClusterId,
    /// The name to show: the display name or the context.
    pub name: String,
    /// Whether it is connected now. A cluster that is not is connected when chosen.
    pub connected: bool,
    /// Whether its read-only flag is on, so a copy there would be refused.
    pub read_only: bool,
}

/// The copy pipeline. Cheap to clone. See the [module docs](super).
///
/// Plain async Rust: the UI runs its futures on the Kubernetes runtime
/// (`oxikube_runtime::spawn_kube`). Every write is a `resource::CopyTo` dispatch on the target
/// cluster, through its guard; this type never touches a writer.
#[derive(Clone)]
pub struct CopyService {
    bus: CommandBus,
    sessions: ClusterSessionManager,
    catalog: Option<ClusterCatalog>,
    namespaces: NamespaceCreator,
    who: Arc<str>,
}

impl std::fmt::Debug for CopyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopyService").finish_non_exhaustive()
    }
}

impl CopyService {
    /// A service dispatching on `bus` as `who` (the local user, for the audit log), from the UI.
    pub fn new(bus: CommandBus, sessions: ClusterSessionManager, who: impl Into<Arc<str>>) -> Self {
        let who: Arc<str> = who.into();
        Self {
            namespaces: NamespaceCreator::new(bus.clone(), who.clone()),
            bus,
            sessions,
            catalog: None,
            who,
        }
    }

    /// Offers the catalog's clusters that are not connected, after the connected ones.
    #[must_use]
    pub fn with_catalog(mut self, catalog: ClusterCatalog) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// The clusters to pick from: the connected ones first (in open order), then the catalog's
    /// others in catalog order.
    pub async fn clusters(&self) -> Vec<ClusterChoice> {
        let mut choices: Vec<ClusterChoice> = self
            .sessions
            .sessions()
            .into_iter()
            .filter(|session| session.is_connected())
            .map(|session| ClusterChoice {
                id: session.id().clone(),
                name: session.title().to_owned(),
                connected: true,
                read_only: session.read_only(),
            })
            .collect();
        let Some(catalog) = &self.catalog else {
            return choices;
        };
        match catalog.load().await {
            Ok(mut entries) => {
                entries.sort_by(|a, b| a.cmp_default(b));
                for entry in entries {
                    if choices.iter().all(|choice| &choice.id != entry.id()) {
                        choices.push(ClusterChoice {
                            id: entry.id().clone(),
                            name: entry.name().to_owned(),
                            connected: false,
                            read_only: false,
                        });
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "the copy picker could not list the contexts"),
        }
        choices
    }

    /// Connects `cluster` unless it is connected (opening it from the catalog first).
    ///
    /// # Errors
    ///
    /// [`CopyError::NotConnected`] when the connection does not come up.
    pub async fn connect(&self, cluster: &ClusterId) -> Result<(), CopyError> {
        if self.sessions.get(cluster).is_some_and(|s| s.is_connected()) {
            return Ok(());
        }
        let name = |sessions: &ClusterSessionManager| {
            sessions
                .get(cluster)
                .map_or_else(|| cluster.to_string(), |s| s.title().to_owned())
        };
        match self.sessions.connect(cluster).await {
            Ok(state) if state.phase().is_connected() => Ok(()),
            Ok(_) | Err(_) => Err(CopyError::NotConnected(name(&self.sessions))),
        }
    }

    /// The namespaces of `cluster`, sorted, for the namespace step.
    ///
    /// # Errors
    ///
    /// [`CopyError::NotConnected`] without a connection, [`CopyError::Failed`] when the list
    /// cannot be read (no right to list namespaces: the picker then asks for a typed name).
    pub async fn namespaces(&self, cluster: &ClusterId) -> Result<Vec<String>, CopyError> {
        self.connect(cluster).await?;
        let reader = self
            .sessions
            .get(cluster)
            .and_then(|session| session.resources())
            .ok_or_else(|| CopyError::NotConnected(cluster.to_string()))?;
        let mut names = Vec::new();
        let mut options = ListOptions {
            limit: Some(500),
            ..ListOptions::default()
        };
        for _ in 0..MAX_NAMESPACE_PAGES {
            let page = reader
                .list_metadata(&Gvk::namespace(), None, &options)
                .await
                .map_err(|error| CopyError::Failed(error.message().to_owned()))?;
            names.extend(page.items.iter().map(|meta| meta.name.to_string()));
            match page.continue_token {
                Some(token) => options.continue_token = Some(token),
                None => break,
            }
        }
        names.sort();
        Ok(names)
    }

    /// Creates the namespace `name` on `cluster`: its own guarded `namespace::Create`, never
    /// done silently as part of a copy.
    ///
    /// # Errors
    ///
    /// [`CopyError::ReadOnly`] on a read-only cluster, the create's own refusal otherwise.
    pub async fn create_namespace(
        &self,
        cluster: &ClusterId,
        name: &str,
    ) -> Result<String, CopyError> {
        self.connect(cluster).await?;
        self.namespaces
            .create(cluster, name, Default::default())
            .await
            .map_err(|error| self.classify_oxi(&error, name))
    }

    /// The target's dry run of `request`: the object as it would be stored next to the one there
    /// (if any), what the sanitiser dropped and the references the target lacks. Connects the
    /// target on demand. Nothing is written; the source is read (and the read audited on the
    /// source cluster).
    ///
    /// An object that exists is previewed as an overwrite, so the diff shows what changes;
    /// [`CopyPreview::exists`] says the user must choose ([`CopyChoice`]) before
    /// [`copy`](Self::copy). Fields other managers own show in [`CopyPreview::conflicts`].
    ///
    /// # Errors
    ///
    /// [`CopyError::ReadOnly`] when the target is read-only (nothing is read), the kind or
    /// apiVersion or namespace is missing on the target, the source cannot be read, or the
    /// target rejects the object.
    pub async fn preview(&self, request: CopyRequest) -> Result<CopyPreview, CopyError> {
        self.connect(&request.target.cluster).await?;
        self.connect(&request.source.cluster).await?;
        let first = self.dry_run(&request, false).await;
        let (output, conflicts) = match first {
            Ok(output) => (output, Vec::new()),
            // The preview shows the force path too: a dry run writes nothing.
            Err(CopyError::Conflicts(conflicts)) => {
                (self.dry_run(&request, true).await?, conflicts)
            }
            Err(error) => return Err(error),
        };
        let data = output.data.unwrap_or(Value::Null);
        Ok(self.parse_preview(request, data, conflicts))
    }

    async fn dry_run(
        &self,
        request: &CopyRequest,
        force: bool,
    ) -> Result<CommandOutput, CopyError> {
        let command = self.command(request, true, force);
        let context = self.context(request).with_dry_run(true);
        // A dry run changes nothing, so the review answers its confirmation: the click, and for
        // a type-the-name tier the name itself. The copy's own confirmation is the user's.
        let outcome = self
            .bus
            .execute(command, context, &DryRunAnswer)
            .await
            .map_err(|error| self.classify(request, error))?;
        match outcome {
            Outcome::Completed(output) => Ok(output),
            Outcome::NeedsConfirmation(_) => Err(CopyError::Unconfirmed),
        }
    }

    fn parse_preview(
        &self,
        request: CopyRequest,
        mut data: Value,
        conflicts: Vec<FieldConflict>,
    ) -> CopyPreview {
        let take = |data: &mut Value, key: &str| data.get_mut(key).map(Value::take);
        let target = data
            .get("target")
            .cloned()
            .and_then(|target| serde_json::from_value(target).ok())
            .unwrap_or_else(|| request.target.clone());
        let text = |value: &Value, key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let missing = data
            .get("missing")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|reference| MissingReference {
                kind: text(reference, "kind"),
                name: text(reference, "name"),
                namespace: reference
                    .get("namespace")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
            .collect();
        let removed = data
            .get("removed")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|path| path.as_str().map(str::to_owned))
            .collect();
        let tier = self.tier(&request);
        CopyPreview {
            live: take(&mut data, "live").filter(|live| !live.is_null()),
            result: take(&mut data, "object").unwrap_or(Value::Null),
            conflicts,
            removed,
            missing,
            tier,
            target,
            request,
        }
    }

    /// The confirmation the target's guard asks of a plain copy of `request`.
    fn tier(&self, request: &CopyRequest) -> ConfirmTier {
        let command = self.command(request, false, false);
        command::lookup(CommandId::RESOURCE_COPY_TO).map_or(ConfirmTier::TypeName, |meta| {
            crate::guard::policy::confirm_tier_for(meta, &command)
        })
    }

    /// Copies what `preview` showed. `choice` says what to do about an object that exists
    /// (required exactly when [`CopyPreview::exists`]); `confirm` answers the guard's confirmation
    /// of the target at its tier (the review's click, the typed name for a cluster-scoped kind).
    ///
    /// # Errors
    ///
    /// [`CopyError::Exists`] when the object exists and `choice` is `None`;
    /// [`CopyError::InvalidName`] for a bad rename; [`CopyError::ReadOnly`], [`Declined`]
    /// and [`Conflicts`] (an overwrite without force over fields other managers own) as the
    /// target's guard and server answer; else as [`preview`](Self::preview).
    ///
    /// [`Declined`]: CopyError::Declined
    /// [`Conflicts`]: CopyError::Conflicts
    pub async fn copy(
        &self,
        preview: &CopyPreview,
        choice: Option<CopyChoice>,
        confirm: Arc<dyn ConfirmationProvider>,
    ) -> Result<CopyOutcome, CopyError> {
        let (request, overwrite, force) = match choice {
            Some(CopyChoice::Skip) => {
                return Ok(CopyOutcome::Skipped {
                    target: preview.target.clone(),
                });
            }
            Some(CopyChoice::Overwrite { force }) => (preview.request.clone(), true, force),
            Some(CopyChoice::Rename(name)) => {
                let label = super::handler::name_is_label(&preview.request.target.gvk);
                validate_name(&name, label).map_err(CopyError::InvalidName)?;
                let renamed = preview.request.renamed(&name);
                // The new name may be taken too: the review of it decides.
                let again = self.preview(renamed.clone()).await?;
                if again.exists() {
                    return Err(CopyError::Exists(again.label()));
                }
                (renamed, false, false)
            }
            None if preview.exists() => return Err(CopyError::Exists(preview.label())),
            None => (preview.request.clone(), false, false),
        };
        let command = self.command(&request, overwrite, force);
        let outcome = self
            .bus
            .execute(command, self.context(&request), confirm.as_ref())
            .await
            .map_err(|error| self.classify(&request, error))?;
        let Outcome::Completed(output) = outcome else {
            return Err(CopyError::Unconfirmed);
        };
        let data = output.data.unwrap_or(Value::Null);
        let target = data
            .get("target")
            .cloned()
            .and_then(|target| serde_json::from_value(target).ok())
            .unwrap_or_else(|| request.target.clone());
        Ok(CopyOutcome::Copied {
            target,
            created: data.get("created").and_then(Value::as_bool).unwrap_or(true),
            message: output.message.unwrap_or_default(),
        })
    }

    fn command(&self, request: &CopyRequest, overwrite: bool, force: bool) -> Command {
        Command::ResourceCopyTo {
            source: request.source.clone(),
            target: request.target.clone(),
            overwrite,
            force,
        }
    }

    /// The dispatch context of a copy: the target cluster's, as the user from the UI.
    fn context(&self, request: &CopyRequest) -> DispatchContext {
        DispatchContext::new(Initiator::Ui, self.who.clone())
            .with_cluster(request.target.cluster.clone())
    }

    fn classify(&self, request: &CopyRequest, error: DispatchError) -> CopyError {
        match error {
            DispatchError::ReadOnly { context, .. } => CopyError::ReadOnly {
                context: context.to_string(),
            },
            DispatchError::Declined(_) => CopyError::Declined,
            other => self.classify_oxi(
                &OxiError::from(other),
                &crate::actions::object_label(&request.target),
            ),
        }
    }

    fn classify_oxi(&self, error: &OxiError, label: &str) -> CopyError {
        if let Some(refusal) = Refusal::of(error) {
            return match refusal.clone() {
                Refusal::KindNotServed(message) => CopyError::KindNotServed(message),
                Refusal::VersionNotServed(message) => CopyError::VersionNotServed(message),
                Refusal::NamespaceMissing(namespace) => CopyError::NamespaceMissing(namespace),
                Refusal::SourceUnavailable(message) => CopyError::SourceUnavailable(message),
                Refusal::InvalidName(message) => CopyError::InvalidName(message),
            };
        }
        if let Some(details) = error.conflict_details() {
            match details.reason {
                ConflictReason::FieldOwnership => {
                    return CopyError::Conflicts(
                        details
                            .causes
                            .iter()
                            .map(|cause| FieldConflict {
                                field: cause.field.clone(),
                                manager: cause.manager.clone(),
                                applied_at: None,
                            })
                            .collect(),
                    );
                }
                ConflictReason::AlreadyExists => return CopyError::Exists(label.to_owned()),
                ConflictReason::StaleVersion | ConflictReason::Other => {}
            }
        }
        if error.kind() == oxikube_domain::ErrorKind::Validation {
            return CopyError::Invalid {
                message: error.message().to_owned(),
                causes: error
                    .validation_details()
                    .map(|details| details.causes.clone())
                    .unwrap_or_default(),
            };
        }
        CopyError::Failed(error.to_string())
    }
}

/// Answers the confirmation of a copy's dry run: the click, and the expected name for a
/// type-the-name tier. The dry run changes nothing.
struct DryRunAnswer;

#[async_trait]
impl ConfirmationProvider for DryRunAnswer {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match &request.expected_name {
            Some(name) => Decision::typed(name.clone()),
            None => Decision::confirm(),
        }
    }
}

/// Answers the guard's confirmation of the real copy from the review: the click, and for a
/// type-the-name tier what the user typed (the guard compares it).
#[derive(Debug, Clone, Default)]
pub struct ReviewAnswer {
    typed: Option<String>,
}

impl ReviewAnswer {
    /// The review's click.
    pub fn click() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The click and the name the user typed.
    pub fn typed(name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            typed: Some(name.into()),
        })
    }
}

#[async_trait]
impl ConfirmationProvider for ReviewAnswer {
    async fn confirm(&self, request: &ConfirmationRequest) -> Decision {
        match (&request.expected_name, &self.typed) {
            (Some(_), Some(typed)) => Decision::typed(typed.clone()),
            (Some(_), None) => Decision::Decline,
            (None, _) => Decision::confirm(),
        }
    }
}
