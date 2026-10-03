//! `oxikube_domain` — layer: `domain`.
//!
//! Dependency-free core: ids (ClusterId, ContextName, Gvk/Gvr, ResourceRef), the thin Resource model (metadata + raw JSON), typed view-models, NamespaceSelection, Command/Capability vocabulary, LogLine/Event/MetricsSample/AuditRecord/ContextBlock, Quantity + Age, error taxonomy, safety (Risk, MutationIntent), redaction.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
