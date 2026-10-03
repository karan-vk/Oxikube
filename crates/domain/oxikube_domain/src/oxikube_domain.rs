//! `oxikube_domain` — layer: `domain`.
//!
//! Core with no internal dependencies and no I/O: ids (ClusterId, ContextName, Gvk/Gvr, ResourceRef), the thin Resource model (metadata + raw JSON), typed view-models, NamespaceSelection, Command/Capability vocabulary, LogLine/Event/MetricsSample/AuditRecord/ContextBlock, Quantity + Age, error taxonomy, safety (Risk, MutationIntent), redaction.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod age;
pub mod command;
pub mod error;
pub mod ids;
pub mod kinds;
pub mod quantity;
pub mod resource;
pub mod safety;
pub mod session;

pub use age::{Age, AgeStyle};
pub use command::{
    Capabilities, Capability, Command, CommandId, CommandMeta, CommandScope, Propagation,
};
pub use error::{ErrorKind, OxiError, OxiResult};
pub use quantity::{Quantity, QuantityError, QuantityFormat};
pub use resource::{ObjectMeta, OwnerRef, Resource, ResourceError};
pub use safety::{ConfirmTier, Initiator, Risk};
