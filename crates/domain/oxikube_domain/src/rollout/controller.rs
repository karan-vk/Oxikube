//! A StatefulSet's or DaemonSet's history (its ControllerRevisions) and the rollback to one.

use serde_json::Value;

use super::selector::is_controlled_by;
use super::template::{changes, images_of, normalised, running_template};
use super::{
    PatchFlavor, PlanAction, Revision, RollbackPlan, RolloutKind, change_cause, check_revision,
    find_revision,
};
use crate::{OxiError, OxiResult, Resource};

/// A ControllerRevision's number (`revision`).
fn revision_number(object: &Resource) -> Option<i64> {
    object.get_i64("/revision")
}

/// The ControllerRevisions among `candidates` that `controller` owns, each with its revision,
/// oldest first.
pub fn owned_controller_revisions(
    controller: &Resource,
    candidates: impl IntoIterator<Item = Resource>,
) -> Vec<(i64, Resource)> {
    let mut found: Vec<(i64, Resource)> = candidates
        .into_iter()
        .filter(|revision| is_controlled_by(revision, controller))
        .filter_map(|revision| Some((revision_number(&revision)?, revision)))
        .collect();
    found.sort_by_key(|(revision, _)| *revision);
    found
}

/// The pod template a ControllerRevision restores: `data.spec.template`, normalised.
fn stored_template(revision: &Resource) -> Option<Value> {
    revision
        .get("/data/spec/template")
        .filter(|t| t.is_object())
        .map(|t| normalised(t.to_value()))
}

/// The revision `controller` runs now: the newest one whose template is the live one. `None` when
/// none matches (the live template was edited and its revision is not stored yet, or the server
/// defaulted a field the stored copy lacks): nothing is then claimed to be running.
fn current_revision(controller: &Resource, owned: &[(i64, Resource)]) -> Option<i64> {
    let live = running_template(controller)?;
    owned
        .iter()
        .rev()
        .find(|(_, revision)| stored_template(revision).as_ref() == Some(&live))
        .map(|(number, _)| *number)
}

/// The rollout history of a StatefulSet or DaemonSet from its [`owned_controller_revisions`]:
/// newest first, the running one flagged, each with how its template differs from the running one.
/// A ControllerRevision has no change cause (kubectl prints `<none>`) and no replicas.
pub fn controller_history(controller: &Resource, owned: &[(i64, Resource)]) -> Vec<Revision> {
    let current = current_revision(controller, owned);
    let running = running_template(controller);
    owned
        .iter()
        .rev()
        .map(|(number, revision)| {
            let template = stored_template(revision);
            let is_current = current == Some(*number);
            let changes = match (&running, &template, is_current) {
                (Some(running), Some(template), false) => changes(running, template),
                _ => Vec::new(),
            };
            Revision {
                revision: *number,
                name: revision.name().into(),
                change_cause: change_cause(revision),
                images: revision
                    .get("/data/spec/template")
                    .map(images_of)
                    .unwrap_or_default(),
                replicas: None,
                created: revision.meta.creation,
                current: is_current,
                changes,
            }
        })
        .collect()
}

/// Works out `kubectl rollout undo` for a StatefulSet or DaemonSet: restore `to_revision`, or by
/// default the previous one (the second newest). The patch is the ControllerRevision's `data`, a
/// strategic merge patch, sent as it is stored.
///
/// # Errors
///
/// `Validation` for a `to_revision` below 1, a revision not in the history, a controller with no
/// earlier revision, and a ControllerRevision with no `data`.
pub fn plan_controller_rollback(
    kind: RolloutKind,
    controller: &Resource,
    owned: &[(i64, Resource)],
    to_revision: Option<i64>,
) -> OxiResult<RollbackPlan> {
    check_revision(to_revision)?;
    let name = controller.name();
    let Some((to, revision)) = find_revision(owned, to_revision) else {
        return Err(OxiError::validation(match to_revision {
            Some(wanted) => format!(
                "unable to find revision {wanted} of {} {name} in its history",
                kind.kubectl_name()
            ),
            None => "no last revision to roll back to".to_owned(),
        }));
    };
    let data = revision
        .get("/data")
        .filter(|data| data.is_object())
        .map(|data| data.to_value())
        .ok_or_else(|| {
            OxiError::validation(format!(
                "controllerrevision {} has no data",
                revision.name()
            ))
        })?;
    let live = running_template(controller);
    let action = if live.is_some() && stored_template(revision) == live {
        PlanAction::AlreadyCurrent
    } else {
        PlanAction::Patch {
            flavor: PatchFlavor::Strategic,
            body: data,
        }
    };
    Ok(RollbackPlan {
        kind,
        from_revision: current_revision(controller, owned),
        to_revision: *to,
        action,
    })
}
