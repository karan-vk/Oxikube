//! The log commands on the `CommandBus` (E08-S02): `pod::ViewLogs` opens a pod's log view, and
//! the `logs::*` commands change one: `SetRange` (tail, head, since 1m ... 1h), `SelectContainer`,
//! `TogglePrevious`, `ToggleWrap`, `ToggleTimestamps`, `ToggleAutoscroll`, `ToggleFullscreen`.
//!
//! None changes a cluster (no `MutationGuard` tier; all are allowed on a read-only cluster):
//! they read logs or change what a view shows. Each is declared in `oxikube_domain::command`, so
//! it has an MCP tool stub (`k8s.pod_view_logs`, `k8s.logs_toggle_wrap`, ...).
//! [`register_commands`] installs handlers that push a [`LogRequest`] into the window's
//! [`LogCommandSink`]; [`LogViews`] applies it on the UI thread.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`LOG_COMMANDS`], [`LogRequest`], [`LogCommandSink`], [`register_commands`] |
//! | `controller` | [`LogViews`]: opens log views in their cluster's tab and applies the `logs::*` requests to them |

mod controller;
#[cfg(test)]
mod tests;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::log::LogRange;

pub use controller::{LogHost, LogViews, LogViewsDeps};

/// The commands this crate handles.
pub const LOG_COMMANDS: [CommandId; 8] = [
    CommandId::POD_VIEW_LOGS,
    CommandId::LOGS_SET_RANGE,
    CommandId::LOGS_SELECT_CONTAINER,
    CommandId::LOGS_TOGGLE_AUTOSCROLL,
    CommandId::LOGS_TOGGLE_FULLSCREEN,
    CommandId::LOGS_TOGGLE_PREVIOUS,
    CommandId::LOGS_TOGGLE_TIMESTAMPS,
    CommandId::LOGS_TOGGLE_WRAP,
];

/// What a `logs::*` command does to the log views of its target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewChange {
    /// `logs::SetRange`.
    SetRange(LogRange),
    /// `logs::SelectContainer`.
    SelectContainer(String),
    /// `logs::ToggleAutoscroll`.
    ToggleAutoscroll,
    /// `logs::ToggleFullscreen`.
    ToggleFullscreen,
    /// `logs::TogglePrevious`.
    TogglePrevious,
    /// `logs::ToggleTimestamps`.
    ToggleTimestamps,
    /// `logs::ToggleWrap`.
    ToggleWrap,
}

/// A log command, resolved, for the UI thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogRequest {
    /// Open (or show) the log view of the pod `target` (`pod::ViewLogs`).
    Open {
        /// The pod.
        target: ResourceRef,
        /// The container to read; `None` for the pod's default.
        container: Option<String>,
        /// Read the previous (terminated) instance.
        previous: bool,
    },
    /// Change the log views of `target` (`logs::*`).
    Change {
        /// What the views show.
        target: ResourceRef,
        /// The change.
        change: ViewChange,
    },
}

impl LogRequest {
    /// The request `command` stands for (`None` for any other command).
    ///
    /// # Errors
    ///
    /// A validation error for `pod::ViewLogs` of something that is not a pod: the logs of a
    /// workload (its pods merged) arrive with E08-S04.
    pub fn of(command: &Command) -> Result<Option<Self>, OxiError> {
        let change = |target: &ResourceRef, change| {
            Some(LogRequest::Change {
                target: target.clone(),
                change,
            })
        };
        Ok(match command {
            Command::PodViewLogs {
                target,
                container,
                previous,
                ..
            } => {
                if !is_pod(target) {
                    return Err(OxiError::validation(format!(
                        "pod::ViewLogs needs a pod, not a {}",
                        target.gvk.kind
                    )));
                }
                Some(LogRequest::Open {
                    target: target.clone(),
                    container: container.clone(),
                    previous: *previous,
                })
            }
            Command::LogsSetRange { target, range } => change(target, ViewChange::SetRange(*range)),
            Command::LogsSelectContainer { target, container } => {
                change(target, ViewChange::SelectContainer(container.clone()))
            }
            Command::LogsToggleAutoscroll { target } => {
                change(target, ViewChange::ToggleAutoscroll)
            }
            Command::LogsToggleFullscreen { target } => {
                change(target, ViewChange::ToggleFullscreen)
            }
            Command::LogsTogglePrevious { target } => change(target, ViewChange::TogglePrevious),
            Command::LogsToggleTimestamps { target } => {
                change(target, ViewChange::ToggleTimestamps)
            }
            Command::LogsToggleWrap { target } => change(target, ViewChange::ToggleWrap),
            _ => None,
        })
    }
}

/// Whether `target` is a core `v1` Pod.
pub fn is_pod(target: &ResourceRef) -> bool {
    target.gvk.group.is_empty() && &*target.gvk.kind == "Pod" && target.namespace.is_some()
}

/// A handle on a window's log request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct LogCommandSink {
    tx: UnboundedSender<LogRequest>,
}

impl LogCommandSink {
    /// A sink and the receiver [`LogViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<LogRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. `false` when the window is gone.
    pub fn send(&self, request: LogRequest) -> bool {
        self.tx.unbounded_send(request).is_ok()
    }
}

/// Registers [`LOG_COMMANDS`] on `registry` with handlers that push into `sink`. Call it from
/// the binary's command setup:
/// `registry.install("oxikube_logs_ui", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// The first [`RegisterError`] (a duplicate registration is a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: LogCommandSink,
) -> Result<(), RegisterError> {
    for id in LOG_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = LogRequest::of(&command)?
                    .ok_or_else(|| OxiError::validation("not a log command"))?;
                if sink.send(request) {
                    Ok(CommandOutput::none())
                } else {
                    Err(OxiError::internal("the window with the log views is gone"))
                }
            }
        })?;
    }
    Ok(())
}
