//! The `ingressclass::SetDefault` handler.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::view::network::{DEFAULT_CLASS_ANNOTATION, is_default_class};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ListOptions, Patch, ResourceWriter, WriteOptions};
use serde_json::{Value, json};

use crate::actions::object_label;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::session::ClusterSessionManager;

/// Registers the `ingressclass::SetDefault` handler on `registry` (the app installs it under
/// `oxikube_app::ingress_class`). `sessions` gives it the cluster's reader, to find the class that is
/// the default now.
///
/// The handler runs only after the guard let the command through: read-only check, confirmation
/// and audit precondition are the guard's. A dispatch that is itself a dry run sends every patch
/// as a server dry run and changes nothing.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::INGRESS_CLASS_SET_DEFAULT).ok_or(
        RegisterError::Undeclared(CommandId::INGRESS_CLASS_SET_DEFAULT),
    )?;
    registry.register(meta, move |command: Command, cx: HandlerContext| {
        let sessions = sessions.clone();
        async move { handle(command, cx, sessions).await }
    })
}

async fn handle(
    command: Command,
    cx: HandlerContext,
    sessions: ClusterSessionManager,
) -> OxiResult<CommandOutput> {
    let Command::IngressClassSetDefault {
        target,
        clear_previous,
    } = command
    else {
        return Err(OxiError::validation(
            "not an ingressclass::SetDefault command",
        ));
    };
    if &*target.gvk.group != "networking.k8s.io" || &*target.gvk.kind != "IngressClass" {
        return Err(OxiError::validation(format!(
            "{} is not an IngressClass",
            object_label(&target)
        )));
    }
    let mutation = cx.require_mutation()?;
    let options = mutation.write_options();
    let writer = mutation.writer();
    // Read first, so a failed read changes nothing.
    let others = if clear_previous {
        other_defaults(&sessions, mutation.cluster(), &target).await?
    } else {
        Vec::new()
    };
    writer
        .patch(
            &target.gvk,
            None,
            &target.name,
            &annotation(Some("true")),
            &options,
        )
        .await?;
    // The new default is set; now the old ones go. A failure here leaves two defaults, which the
    // message says plainly.
    let mut cleared = Vec::new();
    for other in &others {
        clear(writer, &target.gvk, other, &options)
            .await
            .map_err(|error| {
                OxiError::new(
                    error.kind(),
                    format!(
                        "{} is now the default, but the default flag of {other} could not be \
                         removed: {}",
                        target.name,
                        error.message()
                    ),
                )
            })?;
        cleared.push(other.clone());
    }
    let object = object_label(&target);
    let message = match (mutation.dry_run(), cleared.is_empty()) {
        (true, _) => format!("Dry run: {object} can be made the default"),
        (false, true) => format!("{object} is the default Ingress class"),
        (false, false) => format!(
            "{object} is the default Ingress class; cleared {}",
            cleared.join(", ")
        ),
    };
    Ok(CommandOutput {
        message: Some(message),
        data: Some(json!({
            "outcome": if mutation.dry_run() { "dry_run" } else { "default_set" },
            "cleared": cleared,
        })),
    })
}

/// The merge patch that sets (`Some`) or removes (`None`) the default annotation.
fn annotation(value: Option<&str>) -> Patch {
    let value = value.map_or(Value::Null, |v| Value::String(v.to_owned()));
    Patch::merge(json!({"metadata": {"annotations": {DEFAULT_CLASS_ANNOTATION: value}}}))
}

async fn clear(
    writer: &dyn ResourceWriter,
    gvk: &Gvk,
    name: &str,
    options: &WriteOptions,
) -> OxiResult<()> {
    writer
        .patch(gvk, None, name, &annotation(None), options)
        .await
        .map(|_| ())
}

/// The names of the classes other than `target` that are the default now.
async fn other_defaults(
    sessions: &ClusterSessionManager,
    cluster: &ClusterId,
    target: &ResourceRef,
) -> OxiResult<Vec<String>> {
    let reader = sessions
        .get(cluster)
        .and_then(|session| session.resources())
        .ok_or_else(|| OxiError::internal("the cluster is not connected"))?;
    let page = reader
        .list(&target.gvk, None, &ListOptions::default())
        .await?;
    Ok(page
        .items
        .iter()
        .filter(|class| is_default_class(class) && *class.meta.name != *target.name)
        .map(|class| class.meta.name.to_string())
        .collect())
}
