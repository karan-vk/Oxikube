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
//! | [`command`] | [`Command`], [`CommandId`], [`CommandMeta`], [`Capability`] |
//! | [`safety`], [`audit`] | [`Risk`], [`ConfirmTier`], [`Initiator`], [`audit::AuditRecord`] |
//! | [`log`], [`event`], [`metrics`] | telemetry-free records: `LogLine`, `Event`, `MetricsSample` |
//! | [`agent`] | [`ContextBlock`](agent::ContextBlock), the bounded context handed to agents |
//! | [`error`] | [`OxiError`], [`ErrorKind`], [`OxiResult`] |
//!
//! There is no redaction module yet: adapters redact before building records and
//! errors, and a shared `redact` module is planned (E19-S10). The domain never
//! redacts and never does I/O.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

#![deny(missing_docs)]

pub mod age;
pub mod agent;
pub mod audit;
mod bounds;
pub mod command;
pub mod error;
pub mod event;
pub mod ids;
pub mod kinds;
pub mod log;
pub mod metrics;
pub mod quantity;
pub mod resource;
pub mod safety;
pub mod session;
pub mod view;

pub use age::{Age, AgeStyle};
pub use command::{
    Capabilities, Capability, Command, CommandId, CommandMeta, CommandScope, Propagation,
};
pub use error::{ErrorKind, OxiError, OxiResult};
pub use quantity::{Quantity, QuantityError, QuantityFormat};
pub use resource::{ObjectMeta, OwnerRef, Resource, ResourceError};
pub use safety::{ConfirmTier, Initiator, Risk};
pub use view::{
    ContainerSummary, CronJobSummary, JobSummary, NodeSummary, PodSummary, ViewError,
    WorkloadSummary,
};
