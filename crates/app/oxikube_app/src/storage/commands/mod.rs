//! The storage command handlers: `pvc::Resize` and `storageclass::SetDefault`.
//!
//! Both are mutating commands, so the [`MutationGuard`](crate::MutationGuard) has already done
//! the read-only check, asked for the confirmation and armed the audit record when a handler
//! runs; a handler writes only through the guard's [`Mutation`](crate::guard::Mutation). Each
//! reads the live objects first (the claim and its class; the classes), decides with the pure
//! rules of `oxikube_domain::view::storage`, and rehearses its patches with the server's dry run
//! before the real ones, so a refusal (admission, RBAC, "only dynamically provisioned claims
//! can be resized") comes back before anything changes. A dispatch that is itself a dry run
//! stops after the rehearsal.

mod default_class;
mod resize;

use oxikube_domain::command::{self, CommandId};
use oxikube_domain::ids::Gvk;

use crate::command_bus::{CommandRegistry, RegisterError};
use crate::session::ClusterSessionManager;

/// The core `PersistentVolumeClaim` type.
pub(crate) fn claim_gvk() -> Gvk {
    Gvk::new("", "v1", "PersistentVolumeClaim")
}

/// The `storage.k8s.io` `StorageClass` type.
pub(crate) fn class_gvk() -> Gvk {
    Gvk::new("storage.k8s.io", "v1", "StorageClass")
}

/// Registers `pvc::Resize` and `storageclass::SetDefault` on `registry`; they read through the
/// sessions of `sessions` and write through the guard's permit.
///
/// # Errors
///
/// A [`RegisterError`] when either command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    let resize_meta = *command::lookup(CommandId::PVC_RESIZE)
        .ok_or(RegisterError::Undeclared(CommandId::PVC_RESIZE))?;
    let sessions_for_resize = sessions.clone();
    registry.register(resize_meta, move |command, cx| {
        resize::handle(command, cx, sessions_for_resize.clone())
    })?;
    let default_meta = *command::lookup(CommandId::STORAGECLASS_SET_DEFAULT).ok_or(
        RegisterError::Undeclared(CommandId::STORAGECLASS_SET_DEFAULT),
    )?;
    registry.register(default_meta, move |command, cx| {
        default_class::handle(command, cx, sessions.clone())
    })
}
