//! `oxikube_domain` — layer: `domain`.
//!
//! Core with no internal dependencies and no I/O: ids (ClusterId, ContextName, Gvk/Gvr, ResourceRef), the thin Resource model (metadata + raw JSON), typed view-models, NamespaceSelection, Command/Capability vocabulary, LogLine/Event/MetricsSample/AuditRecord/ContextBlock, Quantity + Age, error taxonomy, safety (Risk, MutationIntent), redaction.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod error;
pub mod ids;
pub mod kinds;

pub use error::{ErrorKind, OxiError, OxiResult};
