//! The command and capability vocabulary: [`CommandId`], [`CommandMeta`],
//! [`Capabilities`] and the [`Command`] payload enum.
//!
//! This module only *describes* user actions. Dispatch (`CommandBus`),
//! handlers and the `MutationGuard` pipeline live in `oxikube_app`; keymap
//! files live in `oxikube_keymap`; MCP tool definitions are built from this
//! vocabulary by `oxikube_app::tools`.
//!
//! # One name per action
//!
//! The command id is the keymap action name, the serde `type` tag of the
//! [`Command`] payload, and (through [`CommandId::tool_name`]) the MCP tool
//! name. A keymap binding, a palette entry and an agent tool call therefore all
//! resolve to the same [`Command`] value:
//!
//! ```text
//! keymap action             Command                                   tool name
//! ------------------------  ----------------------------------------  ----------------------------
//! cluster::ToggleReadOnly   Command::ClusterToggleReadOnly { .. }     app.cluster_toggle_read_only
//! workload::Scale           Command::WorkloadScale { target, replicas }  k8s.workload_scale
//! ```
//!
//! A tool call `k8s.workload_scale` with arguments `{target, replicas}` is the
//! JSON `{"type":"workload::Scale","target":...,"replicas":3}`; a keymap entry
//! `{"action":"workload::Scale","args":{...}}` carries the same fields. Renaming
//! an id is a breaking change for both.

mod capability;
mod id;
mod kubeconfig;
mod meta;
mod payload;
mod registry;
mod risk;

pub use capability::{Capabilities, Capability, UnknownCapability};
pub use id::{CommandId, UnknownCommandId, is_well_formed};
pub use kubeconfig::{KubeconfigSourceRef, NewKubeconfigSource, PastedText};
pub use meta::{CommandMeta, CommandScope};
pub use payload::{Command, DEFAULT_DEBUG_COMMAND, DEFAULT_DEBUG_IMAGE, Propagation};
pub use registry::{COMMANDS, lookup, lookup_str};
pub use risk::delete_risk;
