//! [`ApplyService`]: dry run, preview, apply, through the `CommandBus` and its `MutationGuard`.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{ConflictReason, ErrorKind, OxiError};
use serde_json::Value;

use super::manifest::Manifest;
use super::types::{
    ApplyError, ApplyMethod, ApplyPreview, ApplyReport, ApplyRequest, FieldConflict,
};
use crate::command_bus::{CommandOutput, DispatchContext, DispatchError, Outcome};
use crate::guard::Confirmation;
use crate::{ClusterSessionManager, CommandBus};

/// The one apply pipeline (plan review #397: E20 extends it for many objects). See the
/// [module docs](super).
///
/// Cheap to clone. Plain async Rust: the UI runs its futures on the Kubernetes runtime
/// (`oxikube_runtime::spawn_kube`) and never touches a writer; every request is a
/// `resource::Apply` dispatch through the guard (read-only check, confirmation, audit).
#[derive(Clone)]
pub struct ApplyService {
    pub(super) bus: CommandBus,
    pub(super) sessions: ClusterSessionManager,
    pub(super) who: Arc<str>,
}

impl std::fmt::Debug for ApplyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplyService").finish_non_exhaustive()
    }
}

impl ApplyService {
    /// A service dispatching on `bus` as `who` (the local user, for the audit log), from the UI.
    /// `sessions` gives it the live object's field managers for a conflict list.
    pub fn new(bus: CommandBus, sessions: ClusterSessionManager, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            sessions,
            who: who.into(),
        }
    }

    /// The server's dry run of `request`: the object as it would be stored next to the live one,
    /// for the preview. Nothing is written. Answers the guard's confirmation of the dry run
    /// itself: the user asked for this rehearsal, and it changes nothing.
    ///
    /// # Errors
    ///
    /// [`ApplyError::ReadOnly`] on a read-only cluster (audited as denied, nothing sent),
    /// [`ApplyError::Conflicts`] when fields belong to other managers,
    /// [`ApplyError::Invalid`] when the server rejects the manifest, else
    /// [`ApplyError::Failed`].
    pub async fn preview(&self, request: ApplyRequest) -> Result<ApplyPreview, ApplyError> {
        let command = request.command(None);
        let output = self.dispatch(&request, command, true).await?;
        let mut data = output.data.unwrap_or(Value::Null);
        let target = target_of(&data)?;
        Ok(ApplyPreview {
            method: method_of(&data),
            live: take(&mut data, "live").filter(|live| !live.is_null()),
            result: take(&mut data, "object").unwrap_or(Value::Null),
            resource_version: data
                .get("resource_version")
                .and_then(Value::as_str)
                .map(str::to_owned),
            target,
            request,
        })
    }

    /// Applies what `preview` showed. The preview is the confirmation (risk `Low`): the guard's
    /// request is answered with it, unless the guard asks for more, which nothing confirmed.
    /// The guard re-checks read-only mode before the request is sent, and the handler has the
    /// server rehearse it once more.
    ///
    /// # Errors
    ///
    /// As [`preview`](Self::preview), plus [`ApplyError::Stale`] when the fallback's reviewed
    /// version is out of date (or the object a create was reviewed for exists now) and
    /// [`ApplyError::Unconfirmed`].
    pub async fn apply(&self, preview: &ApplyPreview) -> Result<ApplyReport, ApplyError> {
        let request = &preview.request;
        // A preview that showed a create is applied as one: an object made since is not
        // replaced unseen.
        let command = request.reviewed_command(preview.resource_version.clone(), preview.creates());
        let output = self.dispatch(request, command, false).await?;
        let mut data = output.data.unwrap_or(Value::Null);
        Ok(ApplyReport {
            target: target_of(&data).unwrap_or_else(|_| preview.target.clone()),
            method: method_of(&data),
            object: take(&mut data, "object").unwrap_or(Value::Null),
            message: output.message.unwrap_or_default(),
        })
    }

    async fn dispatch(
        &self,
        request: &ApplyRequest,
        command: Command,
        dry_run: bool,
    ) -> Result<CommandOutput, ApplyError> {
        let context = || {
            DispatchContext::new(Initiator::Ui, self.who.clone())
                .with_cluster(request.cluster.clone())
                .with_dry_run(dry_run)
        };
        let outcome = match self.bus.dispatch(command.clone(), context()).await {
            Ok(Outcome::NeedsConfirmation(asked)) => {
                if asked.tier > ConfirmTier::Simple {
                    let _ = self.bus.decline(asked.token).await;
                    return Err(ApplyError::Unconfirmed);
                }
                let answer = Confirmation::simple(asked.token);
                self.bus
                    .dispatch(command, context().with_confirmation(answer))
                    .await
            }
            other => other,
        };
        match outcome {
            Ok(Outcome::Completed(output)) => Ok(output),
            Ok(Outcome::NeedsConfirmation(_)) => Err(ApplyError::Unconfirmed),
            Err(error) => Err(self.classify(request, error).await),
        }
    }

    /// The [`ApplyError`] for a failed dispatch; a field conflict is given the time each manager
    /// last wrote the object.
    async fn classify(&self, request: &ApplyRequest, error: DispatchError) -> ApplyError {
        let error = match error {
            DispatchError::ReadOnly { context, .. } => {
                return ApplyError::ReadOnly {
                    context: context.to_string(),
                };
            }
            other => OxiError::from(other),
        };
        if let Some(details) = error.conflict_details() {
            match details.reason {
                ConflictReason::FieldOwnership => {
                    let times = self.manager_times(request).await;
                    let conflicts = details
                        .causes
                        .iter()
                        .map(|cause| FieldConflict {
                            field: cause.field.clone(),
                            applied_at: cause
                                .manager
                                .as_deref()
                                .and_then(|m| times.get(m).cloned()),
                            manager: cause.manager.clone(),
                        })
                        .collect();
                    return ApplyError::Conflicts(conflicts);
                }
                // An object made since a preview that showed a create is a change since the
                // review too.
                ConflictReason::StaleVersion | ConflictReason::AlreadyExists => {
                    return ApplyError::Stale(error.message().to_owned());
                }
                ConflictReason::Other => {}
            }
        }
        if error.kind() == ErrorKind::Validation {
            return ApplyError::Invalid {
                message: error.message().to_owned(),
                causes: error
                    .validation_details()
                    .map(|details| details.causes.clone())
                    .unwrap_or_default(),
            };
        }
        ApplyError::Failed(error.to_string())
    }

    /// When each field manager of the live object last wrote it (`managedFields[].time`), newest
    /// per manager. Empty when the object cannot be read.
    async fn manager_times(&self, request: &ApplyRequest) -> HashMap<String, String> {
        let (Ok(manifest), Some(session)) = (
            Manifest::parse(&request.manifest),
            self.sessions.get(&request.cluster),
        ) else {
            return HashMap::new();
        };
        let Some(reader) = session.resources() else {
            return HashMap::new();
        };
        let namespace = manifest
            .resolve_namespace(&session, request.namespace.as_deref())
            .await;
        let Ok(Some(live)) = reader
            .get_opt(&manifest.gvk, namespace.as_deref(), &manifest.name)
            .await
        else {
            return HashMap::new();
        };
        manager_times(&live.to_value())
    }
}

/// Newest `managedFields[].time` per manager of `object`.
pub(crate) fn manager_times(object: &Value) -> HashMap<String, String> {
    let mut times: HashMap<String, String> = HashMap::new();
    let entries = object
        .pointer("/metadata/managedFields")
        .and_then(Value::as_array);
    for entry in entries.into_iter().flatten() {
        let (Some(manager), Some(time)) = (
            entry.get("manager").and_then(Value::as_str),
            entry.get("time").and_then(Value::as_str),
        ) else {
            continue;
        };
        let newest = times.entry(manager.to_owned()).or_default();
        // RFC 3339 times in UTC compare as text.
        if time > newest.as_str() {
            *newest = time.to_owned();
        }
    }
    times
}

/// Moves the field `key` out of `data`, without copying it.
fn take(data: &mut Value, key: &str) -> Option<Value> {
    data.get_mut(key).map(Value::take)
}

fn target_of(data: &Value) -> Result<ResourceRef, ApplyError> {
    data.get("target")
        .cloned()
        .and_then(|target| serde_json::from_value(target).ok())
        .ok_or_else(|| ApplyError::Failed("the apply returned no object".to_owned()))
}

fn method_of(data: &Value) -> ApplyMethod {
    data.get("method")
        .and_then(Value::as_str)
        .and_then(ApplyMethod::parse)
        .unwrap_or(ApplyMethod::ServerSide)
}
