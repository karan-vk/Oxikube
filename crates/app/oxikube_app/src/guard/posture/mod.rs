//! Safety posture: read-only mode, cluster colour and the presets (E06-S09; ADR 0012).
//!
//! The *enforcement* of read-only mode is the guard's ([`MutationGuard`](super::MutationGuard)
//! denies every `mutating` command of a read-only session before confirmation or execution, for
//! every initiator, and re-checks right before each request through the
//! [`ReadOnlyGate`](super::gate)). This module is the other half: the commands that change the
//! posture itself.
//!
//! | Command | Effect | Initiators |
//! |---|---|---|
//! | `cluster::ToggleReadOnly` | sets or flips `clusters.<id>.read_only` | people only (privileged) |
//! | `cluster::SetColour` | sets or clears `clusters.<id>.colour` | everyone |
//! | `cluster::ApplyPreset` | writes a [`ClusterPreset`]'s colour and, for production, read-only on | everyone |
//!
//! None of them touches the cluster, so none is `mutating` and none is blocked by read-only
//! mode (lifting it must be possible). They run through the guard's posture pipeline instead
//! ([`MutationGuard::run_posture`]):
//!
//! 1. **Confirmation**: turning read-only *off* on a cluster flagged production (its colour is
//!    the [`Prod`](ClusterPreset::Prod) red) answers
//!    [`Outcome::NeedsConfirmation`](crate::command_bus::Outcome::NeedsConfirmation) with a
//!    simple confirm first. No hidden state: the flag is read from the colour field.
//! 2. **Audit**: one record per attempt, like a mutation. When the command lowers protection
//!    the audit log must be writable first (fail closed); raising protection is never refused
//!    because the log is down.
//! 3. **Handler**: persists through the [`PrefsWriter`] (the comment-preserving settings
//!    writer, supplied by the binary) and applies the change to the live session. Raising
//!    protection goes live first and is persisted second, so a failing disk never leaves a
//!    cluster less protected than the user asked for; lowering is persisted first and applied
//!    second, so a failed write never lowers it.

mod handlers;
mod pipeline;

use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_domain::ClusterColour;
use oxikube_domain::OxiResult;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};

pub use handlers::register_commands;

use crate::session::ClusterSessionManager;

/// The fields a posture command writes to `clusters.<id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrefsPatch {
    /// `read_only`: `Some` writes the value, `None` leaves the field alone.
    pub read_only: Option<bool>,
    /// `colour`: `Some(Some(c))` writes `c`, `Some(None)` removes the field, `None` leaves it.
    pub colour: Option<Option<ClusterColour>>,
}

/// Writes posture fields to the user's settings.
///
/// The app crate has no `gpui` and cannot see the settings store, so the binary implements this
/// on top of `oxikube_settings::ClusterSettings::update_cluster` (comment-preserving, hot
/// reload). The returned future resolves once the value is in the store. It must not block and
/// must not hold a secret: the patch has none.
pub trait PrefsWriter: Send + Sync {
    /// Applies `patch` to `clusters.<cluster>`. `name_hint` (the context name) names a new
    /// block so the user can recognise the opaque id in the file.
    fn write(
        &self,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        patch: PrefsPatch,
    ) -> BoxFuture<'static, OxiResult<()>>;
}

/// What a cluster's safety posture is right now: the live session when it is open, else the
/// settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Posture {
    pub(crate) read_only: bool,
    pub(crate) colour: Option<ClusterColour>,
    /// The kubeconfig context name, when a session is open.
    pub(crate) context: Option<ContextName>,
}

impl Posture {
    pub(crate) fn of(sessions: &ClusterSessionManager, cluster: &ClusterId) -> Self {
        match sessions.get(cluster) {
            Some(session) => Self {
                read_only: session.read_only(),
                colour: session.colour(),
                context: Some(session.context().clone()),
            },
            None => {
                let prefs = sessions.cluster_prefs(cluster);
                Self {
                    read_only: prefs.read_only,
                    colour: prefs.colour,
                    context: None,
                }
            }
        }
    }

    /// What the dialog and the messages call the cluster.
    pub(crate) fn label(&self, cluster: &ClusterId) -> String {
        self.context
            .as_ref()
            .map_or_else(|| cluster.to_string(), ToString::to_string)
    }
}

/// The patch a posture command writes on a cluster whose read-only flag is `read_only` now;
/// `None` for any other command.
pub(crate) fn patch_of(command: &Command, read_only: bool) -> Option<PrefsPatch> {
    match command {
        Command::ClusterToggleReadOnly {
            read_only: want, ..
        } => Some(PrefsPatch {
            read_only: Some(want.unwrap_or(!read_only)),
            colour: None,
        }),
        Command::ClusterSetColour { colour, .. } => Some(PrefsPatch {
            read_only: None,
            colour: Some(*colour),
        }),
        Command::ClusterApplyPreset { preset, .. } => Some(PrefsPatch {
            read_only: preset.read_only(),
            colour: Some(preset.colour()),
        }),
        _ => None,
    }
}

/// Whether running `command` on a cluster with `posture` takes read-only mode from on to off.
pub(crate) fn lowers_protection(command: &Command, posture: &Posture) -> bool {
    posture.read_only
        && patch_of(command, posture.read_only).is_some_and(|p| p.read_only == Some(false))
}

/// A shared handle, for the handlers.
pub(crate) type SharedWriter = Arc<dyn PrefsWriter>;
