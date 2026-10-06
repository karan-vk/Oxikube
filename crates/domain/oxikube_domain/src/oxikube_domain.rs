//! `oxikube_domain` — layer: `domain`.
//!
//! Core with no internal dependencies and no I/O. Vocabulary is documented in
//! `docs/CONTEXT.md`; the modules are:
//!
//! | Module | Holds |
//! |---|---|
//! | [`ids`] | [`ClusterId`](ids::ClusterId), [`ContextName`](ids::ContextName), [`Gvk`](ids::Gvk) / [`Gvr`](ids::Gvr), [`Scope`](ids::Scope), [`ResourceRef`](ids::ResourceRef) |
//! | [`kinds`] | [`ResourceKind`](kinds::ResourceKind), [`Verb`](kinds::Verb), [`VerbSet`](kinds::VerbSet) |
//! | [`resource`] | the thin [`Resource`] model: [`ObjectMeta`] + raw JSON |
//! | [`view`] | typed view-models for core kinds ([`PodSummary`], [`NodeSummary`], ...) |
//! | [`quantity`], [`age`] | [`Quantity`] parsing/formatting and [`Age`] formatting |
//! | [`session`] | the cluster session state machine, `NamespaceSelection`, `WatchScope` |
//! | [`colour`] | [`ClusterColour`], a cluster's `#rrggbb` accent colour |
//! | [`preset`] | [`ClusterPreset`]: the prod / staging / dev / none postures (colour, read-only) |
//! | [`command`] | [`Command`], [`CommandId`], [`CommandMeta`], [`Capability`] |
//! | [`safety`], [`audit`] | [`Risk`], [`ConfirmTier`], [`Initiator`], [`audit::AuditRecord`] |
//! | [`log`], [`event`], [`metrics`] | telemetry-free records: `LogLine`, `Event`, `MetricsSample` |
//! | [`portforward`] | [`ForwardSpec`], [`ForwardStatus`]: what a port-forward targets and how it is doing |
//! | [`agent`] | [`ContextBlock`](agent::ContextBlock), the bounded context handed to agents |
//! | [`error`], [`error_details`] | [`OxiError`], [`ErrorKind`], [`OxiResult`]; [`ConflictDetails`] and [`ValidationDetails`] (field managers and field paths of a rejected write) |
//! | [`redact`] | secret redaction: [`redact::redact`], [`redact::Redacted`], the pattern catalogue |
//!
//! [`redact`] holds the pure secret scrubber (`redact(&str) -> Cow<str>`, `Redacted<T>`). The
//! domain does not call it on its own records: adapters redact before building records and
//! errors, `oxikube_logging` wraps it around every formatted log line, and audit and crash
//! reporting reuse it. The domain never does I/O.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

#![deny(missing_docs)]

pub mod age;
pub mod agent;
pub mod audit;
mod bounds;
pub mod colour;
pub mod command;
pub mod error;
pub mod error_details;
pub mod event;
pub mod ids;
pub mod kinds;
pub mod log;
pub mod metrics;
pub mod portforward;
pub mod preset;
pub mod quantity;
pub mod redact;
pub mod resource;
pub mod safety;
pub mod session;
pub mod view;

pub use age::{Age, AgeStyle};
pub use colour::{ClusterColour, InvalidColour};
pub use command::{
    Capabilities, Capability, Command, CommandId, CommandMeta, CommandScope, Propagation,
};
pub use error::{ErrorKind, OxiError, OxiResult};
pub use error_details::{ConflictDetails, ConflictReason, FieldCause, ValidationDetails};
pub use portforward::{ForwardPort, ForwardSpec, ForwardStatus};
pub use preset::ClusterPreset;
pub use quantity::{Quantity, QuantityError, QuantityFormat};
pub use resource::{ObjectMeta, OwnerRef, Resource, ResourceError};
pub use safety::{ConfirmTier, Initiator, Risk};
pub use view::{
    ContainerSummary, CronJobSummary, JobSummary, NodeSummary, PodSummary, ViewError,
    WorkloadSummary,
};
