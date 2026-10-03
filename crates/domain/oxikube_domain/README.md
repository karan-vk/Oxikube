# oxikube_domain

**Layer:** `domain`

Dependency-free core: ids (ClusterId, ContextName, Gvk/Gvr, ResourceRef), the thin Resource model (metadata + raw JSON), typed view-models, NamespaceSelection, Command/Capability vocabulary, LogLine/Event/MetricsSample/AuditRecord/ContextBlock, Quantity + Age, error taxonomy, safety (Risk, MutationIntent), redaction.

## Allowed internal dependencies

- (none)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
