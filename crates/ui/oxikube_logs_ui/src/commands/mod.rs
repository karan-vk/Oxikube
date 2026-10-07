//! The log commands on the `CommandBus` (E08-S02): `pod::ViewLogs` opens a pod's log view,
//! `workload::ViewLogs` (E08-S04) opens the merged log of a workload's or Service's pods, and the
//! `logs::*` commands change one: `SetRange` (tail, head, since 1m ... 1h), `SelectContainer`,
//! `TogglePrevious`, `ToggleWrap`, `ToggleTimestamps`, `ToggleAutoscroll`, `ToggleFullscreen`,
//! `ToggleSource` (E08-S04: switch one pod or container of a merged log off or on), and
//! the search's (E08-S03) `Find`, `NextMatch`, `PreviousMatch`, `ToggleCase`, `ToggleInverse`,
//! `ToggleFilterMode`, `CloseSearch`, the JSON mode's (E08-S05) `ToggleJsonMode`,
//! `ToggleLevel`, `ToggleLine` and `CollapseLine`, (E08-S06) the local actions on what a
//! view holds: `Mark`, `Copy`, `Clear`, `Save`, and (E08-S07) what follows a stream that stopped:
//! `Reconnect` and `FollowReplacement` (switch to the pod that replaced a gone one).
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
use oxikube_app::logs::is_aggregate_kind;
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::log::{LevelChip, LogRange, LogSaveScope};

use crate::view::OpenLogs;
pub use controller::{LogHost, LogViews, LogViewsDeps};

/// The commands this crate handles.
pub const LOG_COMMANDS: [CommandId; 28] = [
    CommandId::POD_VIEW_LOGS,
    CommandId::WORKLOAD_VIEW_LOGS,
    CommandId::LOGS_FOLLOW_REPLACEMENT,
    CommandId::LOGS_RECONNECT,
    CommandId::LOGS_TOGGLE_SOURCE,
    CommandId::LOGS_CLEAR,
    CommandId::LOGS_COPY,
    CommandId::LOGS_MARK,
    CommandId::LOGS_SEND_TO_AGENT,
    CommandId::LOGS_SAVE,
    CommandId::LOGS_CLOSE_SEARCH,
    CommandId::LOGS_FIND,
    CommandId::LOGS_NEXT_MATCH,
    CommandId::LOGS_PREVIOUS_MATCH,
    CommandId::LOGS_TOGGLE_CASE,
    CommandId::LOGS_TOGGLE_FILTER_MODE,
    CommandId::LOGS_TOGGLE_INVERSE,
    CommandId::LOGS_SET_RANGE,
    CommandId::LOGS_SELECT_CONTAINER,
    CommandId::LOGS_TOGGLE_AUTOSCROLL,
    CommandId::LOGS_TOGGLE_FULLSCREEN,
    CommandId::LOGS_TOGGLE_PREVIOUS,
    CommandId::LOGS_TOGGLE_TIMESTAMPS,
    CommandId::LOGS_TOGGLE_WRAP,
    CommandId::LOGS_TOGGLE_JSON_MODE,
    CommandId::LOGS_TOGGLE_LEVEL,
    CommandId::LOGS_TOGGLE_LINE,
    CommandId::LOGS_COLLAPSE_LINE,
];

/// What a `logs::*` command does to the log views of its target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewChange {
    /// `logs::Clear`.
    Clear,
    /// `logs::Copy`.
    Copy,
    /// `logs::Mark`.
    Mark,
    /// `logs::SendToAgent`: queue the selection (else the lines on screen) as agent context.
    SendToAgent,
    /// `logs::Save`: offer to write the lines of this scope to a file.
    Save(LogSaveScope),
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
    /// `logs::Find`: open the search bar, searching for the pattern when there is one.
    Find(Option<String>),
    /// `logs::NextMatch`.
    NextMatch,
    /// `logs::PreviousMatch`.
    PreviousMatch,
    /// `logs::ToggleCase`.
    ToggleCase,
    /// `logs::ToggleInverse`.
    ToggleInverse,
    /// `logs::ToggleFilterMode`.
    ToggleFilterMode,
    /// `logs::CloseSearch`.
    CloseSearch,
    /// `logs::ToggleJsonMode`.
    ToggleJsonMode,
    /// `logs::ToggleLevel`.
    ToggleLevel(LevelChip),
    /// `logs::ToggleLine`: the line with this seq.
    ToggleLine(u64),
    /// `logs::CollapseLine`.
    CollapseLine,
    /// `logs::Reconnect`.
    Reconnect,
    /// `logs::FollowReplacement`.
    FollowReplacement,
    /// `logs::ToggleSource`: switch a pod (or one of its containers) of a merged log off or on.
    ToggleSource {
        /// The pod.
        pod: String,
        /// The container; `None` for the whole pod.
        container: Option<String>,
    },
}

/// A log command, resolved, for the UI thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogRequest {
    /// Open (or show) the log view of the pod `target` (`pod::ViewLogs`), or of the workload or
    /// Service `target` (`workload::ViewLogs`): its pods merged.
    Open {
        /// The pod.
        target: ResourceRef,
        /// What to read: container, previous instance, follow, tail length.
        open: OpenLogs,
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
    /// A validation error for `pod::ViewLogs` of something that is not a pod (a workload's logs
    /// are `workload::ViewLogs`), and for `workload::ViewLogs` of something that is not a
    /// Deployment, StatefulSet, DaemonSet, ReplicaSet, Job or Service.
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
                follow,
                previous,
                tail_lines,
            } => {
                if !is_pod(target) {
                    let hint = if is_aggregate_kind(&target.gvk) {
                        " (use workload::ViewLogs for its pods)"
                    } else {
                        ""
                    };
                    return Err(OxiError::validation(format!(
                        "pod::ViewLogs needs a pod, not a {}{hint}",
                        target.gvk.kind
                    )));
                }
                Some(LogRequest::Open {
                    target: target.clone(),
                    open: OpenLogs {
                        container: container.clone(),
                        previous: *previous,
                        follow: *follow,
                        tail_lines: *tail_lines,
                        selector: None,
                    },
                })
            }
            Command::LogsClear { target } => change(target, ViewChange::Clear),
            Command::LogsCopy { target } => change(target, ViewChange::Copy),
            Command::LogsMark { target } => change(target, ViewChange::Mark),
            Command::LogsSendToAgent { target } => change(target, ViewChange::SendToAgent),
            Command::LogsReconnect { target } => change(target, ViewChange::Reconnect),
            Command::LogsFollowReplacement { target } => {
                change(target, ViewChange::FollowReplacement)
            }
            Command::LogsSave { target, scope } => change(target, ViewChange::Save(*scope)),
            Command::WorkloadViewLogs {
                target,
                selector,
                container,
                follow,
                tail_lines,
            } => {
                if !is_aggregate_kind(&target.gvk) || target.namespace.is_none() {
                    return Err(OxiError::validation(format!(
                        "workload::ViewLogs needs a Deployment, StatefulSet, DaemonSet, \
                         ReplicaSet, Job or Service, not a {}",
                        target.gvk.kind
                    )));
                }
                Some(LogRequest::Open {
                    target: target.clone(),
                    open: OpenLogs {
                        container: container.clone(),
                        previous: false,
                        follow: *follow,
                        tail_lines: *tail_lines,
                        selector: selector.clone().filter(|s| !s.trim().is_empty()),
                    },
                })
            }
            Command::LogsToggleSource {
                target,
                pod,
                container,
            } => change(
                target,
                ViewChange::ToggleSource {
                    pod: pod.clone(),
                    container: container.clone(),
                },
            ),
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
            Command::LogsFind { target, pattern } => {
                change(target, ViewChange::Find(pattern.clone()))
            }
            Command::LogsNextMatch { target } => change(target, ViewChange::NextMatch),
            Command::LogsPreviousMatch { target } => change(target, ViewChange::PreviousMatch),
            Command::LogsToggleCase { target } => change(target, ViewChange::ToggleCase),
            Command::LogsToggleInverse { target } => change(target, ViewChange::ToggleInverse),
            Command::LogsToggleFilterMode { target } => {
                change(target, ViewChange::ToggleFilterMode)
            }
            Command::LogsCloseSearch { target } => change(target, ViewChange::CloseSearch),
            Command::LogsToggleJsonMode { target } => change(target, ViewChange::ToggleJsonMode),
            Command::LogsToggleLevel { target, level } => {
                change(target, ViewChange::ToggleLevel(*level))
            }
            Command::LogsToggleLine { target, seq } => change(target, ViewChange::ToggleLine(*seq)),
            Command::LogsCollapseLine { target } => change(target, ViewChange::CollapseLine),
            _ => None,
        })
    }
}

/// Whether `gvk` is the core `v1` Pod.
pub(crate) fn is_pod_gvk(gvk: &Gvk) -> bool {
    gvk.group.is_empty() && &*gvk.kind == "Pod"
}

/// Whether `target` is a pod (they are all namespaced).
fn is_pod(target: &ResourceRef) -> bool {
    is_pod_gvk(&target.gvk) && target.namespace.is_some()
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
