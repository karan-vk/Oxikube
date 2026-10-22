//! Helm in the running app: capability detection, the gate, the `helm::*` commands and the
//! Helm page (E16-S11).
//!
//! * [`controller`] owns the one `HelmCli` and the app's [`HelmGateState`]. It finds the binary
//!   off the UI thread (`spawn_kube`) shortly after start-up, again when `helm.path` changes
//!   (hot reload, no restart) and whenever `helm::Recheck` runs, and publishes what it found.
//! * [`register_commands`] puts `helm::Recheck` and `helm::LocateBinary` on the bus (palette,
//!   the banner's buttons, MCP tools).
//!
//! The Helm page itself (banner and releases) is opened by `mount::helm_releases`. A user reaches
//! it from the sidebar's Helm > Releases entry in a cluster tab, `:helm`, the palette ("Recheck Helm", "Locate Helm Binary") and the `helm` block of
//! `settings.json`. The palette also hides the commands that need the `helm` capability while
//! the gate is not full ([`palette_capabilities`]).

mod controller;
mod tab;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use futures::channel::mpsc;
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::helm::HelmGate;
use oxikube_domain::{Capabilities, OxiResult};
use serde_json::json;

#[cfg(test)]
pub use controller::FileOpener;
pub use controller::install;
pub use tab::HELM_PAGE;

/// What the bus handlers ask the controller (it runs on the UI thread).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Detect the binary again with the current settings.
    Recheck,
    /// The `helm` setting changed: detect again only when `helm.path` did.
    SettingsChanged,
    /// Show the user's settings file so they can set `helm.path`.
    Open(PathBuf),
}

/// Where the handlers send [`Request`]s.
pub type Sink = mpsc::UnboundedSender<Request>;

/// Registers `helm::Recheck` and `helm::LocateBinary`. Neither changes a cluster: the first runs
/// `helm version`, the second creates `settings.json` when missing (with its commented
/// defaults) and shows it.
///
/// # Errors
///
/// A [`RegisterError`] when a command is not declared or already has a handler.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: Sink,
    settings_file: Option<PathBuf>,
) -> Result<(), RegisterError> {
    let tx = sink.clone();
    registry.register(
        declared(CommandId::HELM_RECHECK)?,
        move |_: Command, _: HandlerContext| {
            let tx = tx.clone();
            async move {
                send(&tx, Request::Recheck)?;
                Ok(CommandOutput::message("Looking for Helm"))
            }
        },
    )?;
    registry.register(
        declared(CommandId::HELM_LOCATE_BINARY)?,
        move |_: Command, _: HandlerContext| {
            let (tx, file) = (sink.clone(), settings_file.clone());
            async move {
                let file = file.ok_or_else(|| {
                OxiError::not_found(
                    "this session has no config directory, so there is no settings.json to open",
                )
            })?;
                // One small file, on the bus's worker (never the UI thread).
                ensure_settings_file(&file)?;
                send(&tx, Request::Open(file.clone()))?;
                Ok(CommandOutput::data(json!({
                    "path": file.display().to_string(),
                    "setting": "helm.path",
                })))
            }
        },
    )?;
    Ok(())
}

fn declared(id: CommandId) -> Result<command::CommandMeta, RegisterError> {
    command::lookup(id)
        .copied()
        .ok_or(RegisterError::Undeclared(id))
}

fn send(tx: &Sink, request: Request) -> OxiResult<()> {
    tx.unbounded_send(request)
        .map_err(|_| OxiError::internal("the app is shutting down"))
}

/// Makes sure `file` exists, creating it from the commented template like the first run does.
fn ensure_settings_file(file: &std::path::Path) -> OxiResult<()> {
    oxikube_settings::paths::load_or_create_user_settings(file).map(drop)
}

/// The capabilities the palette should treat the session as having: the session's own, with
/// [`Capabilities::HELM`] following `gate`, so a command that needs it is hidden (and explained
/// under "show all") while Helm is missing.
pub fn palette_capabilities(gate: &HelmGate, session: Capabilities) -> Capabilities {
    gate.apply_to(session)
}
